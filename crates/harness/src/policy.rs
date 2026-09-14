//! The permission policy engine (harness guide §14.3).
//!
//! Maps `(risk × archetype × provenance)` to `allow | ask | deny`. Before this, the
//! platform had approvals but nothing to key them on: every tool looked the same to
//! the policy because no tool declared what it does.
//!
//! Two rules here are the ones worth arguing about, and both come straight from the
//! guide:
//!
//! - **An autonomous agent gets NARROWER standing permissions than an interactive
//!   one.** Absence of a user watching means less authority, never more. The
//!   intuition runs the other way — an unattended agent seems to *need* more leeway
//!   to get anything done — and that intuition is how unattended agents cause
//!   incidents.
//! - **An action proposed right after reading untrusted content is escalated**, no
//!   matter the tier or the archetype. That is the exact shape of a successful
//!   prompt injection: hostile text arrives, and the very next step tries to do
//!   something with reach.

use serde::{Deserialize, Serialize};

use crate::profile::{Archetype, Profile};
use crate::provenance::Provenance;
use crate::registry::Risk;

/// What the harness does with a proposed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Proceed.
    Allow,
    /// Pause the state machine, persist a pending approval, resume on an answer.
    /// A first-class loop state, not an afterthought.
    Ask,
    /// Refuse. The model is told why and what it may do instead.
    Deny,
}

impl Decision {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }
}

/// The decision plus why it was made. The reason is model-facing and audit-facing at
/// once: a refusal the model cannot act on becomes a retry loop.
#[derive(Debug, Clone, Serialize)]
pub struct Ruling {
    pub decision: Decision,
    /// Stable machine code, e.g. `"policy.untrusted_escalation"`.
    pub code: &'static str,
    pub reason: String,
    /// What to do instead. Always present on `Ask` and `Deny`.
    pub fix: String,
}

impl Ruling {
    fn allow() -> Self {
        Self {
            decision: Decision::Allow,
            code: "policy.allow",
            reason: String::new(),
            fix: String::new(),
        }
    }

    fn ask(code: &'static str, reason: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            decision: Decision::Ask,
            code,
            reason: reason.into(),
            fix: fix.into(),
        }
    }

    fn deny(code: &'static str, reason: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            decision: Decision::Deny,
            code,
            reason: reason.into(),
            fix: fix.into(),
        }
    }
}

/// What the harness knows about the step being authorised.
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub tool: String,
    pub risk: Risk,
    /// The highest-untrust provenance read during the PREVIOUS step.
    ///
    /// "Previous", not "current": the injection arrives in step N's tool result and
    /// fires in step N+1's tool call.
    pub prior_provenance: Provenance,
    /// Whether a human is attached to this session right now.
    pub user_present: bool,
    /// Session allowlist: the operator said "always allow this tool this session".
    /// Scoped and logged (§14.3); it never covers destructive or outbound risk.
    pub session_allowlisted: bool,
    /// The approval envelope this action falls under, when it falls under one
    /// (SPEC §15). Supplied by the platform, which owns the classification;
    /// this crate only honours it. `None` means ungated, which is most tools.
    pub approval_envelope: Option<String>,
}

/// Whether this agent is permitted an outbound channel at all.
///
/// Derived from the trifecta audit rather than asked separately, so the two cannot
/// disagree: if the config says the outbound leg is cut, outbound tools are denied.
#[must_use]
pub fn outbound_permitted(profile: &Profile) -> bool {
    profile.trifecta.outbound_channel
}

/// The policy.
#[must_use]
pub fn decide(profile: &Profile, ctx: &ActionContext) -> Ruling {
    // 1. The trifecta decision, enforced. An agent whose config cut the outbound leg
    //    does not get outbound tools, whatever else is true.
    if ctx.risk == Risk::Outbound && !outbound_permitted(profile) {
        return Ruling::deny(
            "policy.outbound_cut",
            format!(
                "{} sends data outside the platform, and this agent's outbound channel is cut",
                ctx.tool
            ),
            "put the result in outputs/ and call deliver; a human publishes it",
        );
    }

    // 2. The approval envelopes (§15, ADR-P2-20). Four actions are gated by
    //    *what they are* rather than by how risky the tool looks: spending,
    //    promotion toward capital, opening the sealed holdout, and writing the
    //    memory later decisions read. None of them depends on attendance, tier
    //    or archetype, and a session allowlist does not cover them — the same
    //    reasoning as destruction, one step further: "always allow" is exactly
    //    the setting somebody reaches for on the fifth promotion of the day.
    if let Some(envelope) = &ctx.approval_envelope {
        return Ruling::ask(
            "policy.approval_envelope",
            format!("{} falls under the `{envelope}` approval envelope", ctx.tool),
            "a human answers this one; the session pauses and resumes where it left off",
        );
    }

    // 3. Untrusted content escalation. Regardless of tier, archetype, or an
    //    allowlist: an action with reach proposed immediately after reading a
    //    stranger's text is the shape of a successful injection.
    if ctx.prior_provenance.is_untrusted() && matches!(ctx.risk, Risk::Destructive | Risk::Outbound)
    {
        return Ruling::ask(
            "policy.untrusted_escalation",
            format!(
                "{} is {} and the previous step read external untrusted content",
                ctx.tool,
                ctx.risk.as_str()
            ),
            "a human confirms this one; say in your request what in the content prompted it",
        );
    }

    // 4. Destructive actions always ask, at every tier. The guide makes this
    //    independent of model tier on purpose — a frontier model deleting the wrong
    //    thing is the same incident as a local one doing it.
    if ctx.risk == Risk::Destructive {
        if ctx.session_allowlisted {
            // Deliberately not honoured. A session allowlist is a convenience for
            // repetitive writes; extending it to destruction is how "always allow"
            // becomes an incident report.
            return Ruling::ask(
                "policy.destructive_not_allowlistable",
                format!(
                    "{} is destructive; session allowlists do not cover destruction",
                    ctx.tool
                ),
                "confirm this call, or use a non-destructive tool",
            );
        }
        return Ruling::ask(
            "policy.destructive",
            format!(
                "{} destroys state that the platform cannot recover",
                ctx.tool
            ),
            "confirm, or pick a tool that writes a new version instead of replacing one",
        );
    }

    // 5. Autonomy narrows authority. An unattended background agent gets LESS than
    //    an interactive one.
    let unattended = !ctx.user_present || profile.archetype == Archetype::Background;
    if unattended && ctx.risk == Risk::Write && !ctx.session_allowlisted {
        return Ruling::ask(
            "policy.unattended_write",
            format!(
                "{} writes state and no human is attached to this session",
                ctx.tool
            ),
            "the write is queued for approval; nothing is lost, the session resumes when it is answered",
        );
    }

    // 6. Outbound with the channel permitted still asks when unattended.
    if ctx.risk == Risk::Outbound && unattended {
        return Ruling::ask(
            "policy.unattended_outbound",
            format!("{} sends data and no human is attached", ctx.tool),
            "confirm the recipient and the payload",
        );
    }

    Ruling::allow()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::frontier_fixture;

    fn ctx(risk: Risk) -> ActionContext {
        ActionContext {
            tool: "some_tool".into(),
            risk,
            prior_provenance: Provenance::ToolInternal,
            user_present: true,
            session_allowlisted: false,
            approval_envelope: None,
        }
    }

    fn interactive_profile() -> Profile {
        let mut p = frontier_fixture();
        p.archetype = Archetype::KnowledgeWorker;
        p
    }

    #[test]
    fn reads_are_allowed_without_ceremony() {
        assert_eq!(
            decide(&interactive_profile(), &ctx(Risk::Read)).decision,
            Decision::Allow
        );
    }

    #[test]
    fn destructive_always_asks_even_at_the_frontier_tier() {
        let r = decide(&interactive_profile(), &ctx(Risk::Destructive));
        assert_eq!(r.decision, Decision::Ask);
        assert!(
            !r.fix.is_empty(),
            "a refusal without a fix becomes a retry loop"
        );
    }

    /// The rule that runs against intuition, so it gets its own test.
    #[test]
    fn an_unattended_agent_has_narrower_authority_than_an_attended_one() {
        let p = interactive_profile();
        let mut attended = ctx(Risk::Write);
        attended.user_present = true;
        assert_eq!(decide(&p, &attended).decision, Decision::Allow);

        let mut unattended = ctx(Risk::Write);
        unattended.user_present = false;
        assert_eq!(
            decide(&p, &unattended).decision,
            Decision::Ask,
            "absence of a user watching means less authority, never more"
        );
    }

    #[test]
    fn a_background_archetype_is_unattended_even_with_a_user_watching() {
        // The archetype is the standing posture; a human happening to have the tab
        // open does not widen it.
        let p = frontier_fixture(); // archetype = Background
        let mut c = ctx(Risk::Write);
        c.user_present = true;
        assert_eq!(decide(&p, &c).decision, Decision::Ask);
    }

    /// The injection shape: hostile text in step N, an action with reach in N+1.
    #[test]
    fn an_action_after_untrusted_content_escalates() {
        let p = interactive_profile();
        let mut c = ctx(Risk::Destructive);
        c.prior_provenance = Provenance::ExternalUntrusted;
        let r = decide(&p, &c);
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.code, "policy.untrusted_escalation");
    }

    #[test]
    fn untrusted_content_does_not_escalate_a_plain_read() {
        let p = interactive_profile();
        let mut c = ctx(Risk::Read);
        c.prior_provenance = Provenance::ExternalUntrusted;
        assert_eq!(
            decide(&p, &c).decision,
            Decision::Allow,
            "escalating every read after any web page would make research impossible"
        );
    }

    #[test]
    fn a_session_allowlist_never_covers_destruction() {
        let p = interactive_profile();
        let mut c = ctx(Risk::Destructive);
        c.session_allowlisted = true;
        let r = decide(&p, &c);
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.code, "policy.destructive_not_allowlistable");
    }

    #[test]
    fn a_session_allowlist_does_cover_repetitive_writes() {
        let p = interactive_profile();
        let mut c = ctx(Risk::Write);
        c.user_present = false;
        c.session_allowlisted = true;
        assert_eq!(decide(&p, &c).decision, Decision::Allow);
    }

    /// The trifecta decision and the policy cannot disagree, because one reads the
    /// other.
    #[test]
    fn an_agent_whose_outbound_leg_is_cut_is_denied_outbound_tools() {
        let p = interactive_profile(); // outbound_channel: false
        let r = decide(&p, &ctx(Risk::Outbound));
        assert_eq!(r.decision, Decision::Deny);
        assert_eq!(r.code, "policy.outbound_cut");
        assert!(r.fix.contains("deliver"));
    }

    #[test]
    fn an_agent_permitted_outbound_still_asks_when_unattended() {
        let mut p = interactive_profile();
        p.trifecta.outbound_channel = true;
        p.trifecta.mitigation = "gated behind deliver".into();
        let mut c = ctx(Risk::Outbound);
        c.user_present = false;
        assert_eq!(decide(&p, &c).decision, Decision::Ask);
        c.user_present = true;
        assert_eq!(decide(&p, &c).decision, Decision::Allow);
    }

    #[test]
    fn every_non_allow_ruling_carries_a_code_and_a_fix() {
        let p = interactive_profile();
        for risk in [Risk::Write, Risk::Destructive, Risk::Outbound] {
            for untrusted in [true, false] {
                let mut c = ctx(risk);
                c.user_present = false;
                c.prior_provenance = if untrusted {
                    Provenance::ExternalUntrusted
                } else {
                    Provenance::ToolInternal
                };
                let r = decide(&p, &c);
                if r.decision != Decision::Allow {
                    assert!(!r.reason.is_empty(), "{risk:?} has no reason");
                    assert!(!r.fix.is_empty(), "{risk:?} has no fix");
                    assert!(r.code.starts_with("policy."));
                }
            }
        }
    }
}

#[cfg(test)]
mod envelope_tests {
    use super::*;
    use crate::profile::{frontier_fixture, Archetype, Profile};

    fn interactive() -> Profile {
        let mut p = frontier_fixture();
        p.archetype = Archetype::KnowledgeWorker;
        p
    }

    fn ctx(tool: &str, envelope: Option<&str>, allowlisted: bool) -> ActionContext {
        ActionContext {
            tool: tool.to_string(),
            risk: Risk::Read,
            prior_provenance: Provenance::ToolInternal,
            user_present: true,
            session_allowlisted: allowlisted,
            approval_envelope: envelope.map(ToString::to_string),
        }
    }

    /// The four §15 envelopes pause the session regardless of how harmless the
    /// tool's *risk* looks. `write_insight` is a `Read`-risk write to a text
    /// store; what makes it gated is that later decisions read it.
    #[test]
    fn an_enveloped_action_always_asks() {
        let ruling = decide(&interactive(), &ctx("write_insight", Some("tier3_memory"), false));
        assert_eq!(ruling.decision, Decision::Ask);
        assert_eq!(ruling.code, "policy.approval_envelope");
        assert!(ruling.fix.contains("pauses"), "{}", ruling.fix);
    }

    /// A session allowlist is a convenience for repetitive writes. Extending it
    /// to promotion or spending is how "always allow" becomes an incident.
    #[test]
    fn a_session_allowlist_does_not_cover_an_envelope() {
        let ruling = decide(&interactive(), &ctx("promote_model", Some("promotion"), true));
        assert_eq!(ruling.decision, Decision::Ask);
        assert_eq!(ruling.code, "policy.approval_envelope");
    }

    /// Attendance does not either: a human at the keyboard is not the same as a
    /// human who agreed to this.
    #[test]
    fn attendance_does_not_cover_an_envelope() {
        let mut background = interactive();
        background.archetype = Archetype::Background;
        let attended = decide(&interactive(), &ctx("open_vault", Some("sealed_holdout"), false));
        let unattended = decide(&background, &ctx("open_vault", Some("sealed_holdout"), false));
        assert_eq!(attended.decision, Decision::Ask);
        assert_eq!(unattended.decision, Decision::Ask);
        assert_eq!(attended.code, unattended.code);
    }

    /// And everything outside the four runs free. The list is short precisely so
    /// that the prompts that do appear still mean something.
    #[test]
    fn an_unenveloped_read_runs_without_asking() {
        assert_eq!(
            decide(&interactive(), &ctx("list_instruments", None, false)).decision,
            Decision::Allow
        );
    }

    /// An outbound channel that was cut stays cut. A `Deny` is a harder answer
    /// than an `Ask`, and the envelope must not soften it into "ask a human".
    #[test]
    fn a_cut_outbound_channel_still_denies() {
        let p = interactive();
        let mut c = ctx("publish_report", Some("spend"), false);
        c.risk = Risk::Outbound;
        if !outbound_permitted(&p) {
            let ruling = decide(&p, &c);
            assert_eq!(ruling.decision, Decision::Deny);
            assert_eq!(ruling.code, "policy.outbound_cut");
        }
    }
}

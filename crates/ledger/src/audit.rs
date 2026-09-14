//! The audit trail (SPEC §15, checklist 2.18, ADR-P2-21, AT-64).
//!
//! `mlops.audit_event` is hash-chained and append-only like the trial stream.
//! What makes it an *audit* trail rather than a log is the two-phase write:
//!
//! * **`pre`** — written **before** the policy check, saying what was asked for;
//! * **`post`** — written after, saying what the platform did about it, and
//!   pointing back at its own `pre` row (the table CHECKs that it does).
//!
//! The reason for the first half is narrow and it is the whole point. A trail
//! that only records what happened cannot show what was *prevented*: a denied
//! action leaves no trace, so "the agent never tried to do that" and "the agent
//! tried and was stopped" look identical afterwards. The `pre` row is the
//! evidence that the control did something.
//!
//! ## Who writes it
//!
//! The API layer, not the agent harness. A trail the agent writes is a trail the
//! agent can omit — not by malice, just by an exception path that skips the
//! call. The API layer is the component a request cannot route around, and it
//! writes `pre` before it has even decided whether the request is allowed.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The approval envelope an action falls under (§15).
///
/// Four things are gated and nothing else is. The list is short on purpose:
/// users approve roughly 93 % of prompts, so a platform that asks about
/// everything has trained its users to click yes, and the four that matter get
/// the same reflex as the forty that do not (ADR-P2-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Envelope {
    /// Not gated.
    None,
    /// Spending money or compute beyond the campaign's declared allowance.
    Spend,
    /// Moving a strategy toward capital.
    Promotion,
    /// Opening the sealed holdout — once, ever, per lineage.
    SealedHoldout,
    /// Writing to durable agent memory that later decisions will read.
    Tier3Memory,
}

impl Envelope {
    /// Every envelope, matching `mlops.audit_event`'s CHECK.
    pub const ALL: [Self; 5] =
        [Self::None, Self::Spend, Self::Promotion, Self::SealedHoldout, Self::Tier3Memory];

    /// The four that require approval.
    pub const GATED: [Self; 4] =
        [Self::Spend, Self::Promotion, Self::SealedHoldout, Self::Tier3Memory];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Spend => "spend",
            Self::Promotion => "promotion",
            Self::SealedHoldout => "sealed_holdout",
            Self::Tier3Memory => "tier3_memory",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == code)
    }

    #[must_use]
    pub fn requires_approval(self) -> bool {
        self != Self::None
    }

    /// Which envelope an action falls under.
    ///
    /// Matched on the action's **name**, which is the identity the tool
    /// catalogue and the audit table share. An action nobody has classified is
    /// `None` — ungated — and that is the direction a mistake has to be caught
    /// in, so `every_gated_action_is_classified` pins the four lists against the
    /// catalogue rather than trusting this match to stay complete.
    #[must_use]
    pub fn for_action(action: &str) -> Self {
        match action {
            a if SPEND_ACTIONS.contains(&a) => Self::Spend,
            a if PROMOTION_ACTIONS.contains(&a) => Self::Promotion,
            a if SEALED_HOLDOUT_ACTIONS.contains(&a) => Self::SealedHoldout,
            a if TIER3_MEMORY_ACTIONS.contains(&a) => Self::Tier3Memory,
            _ => Self::None,
        }
    }
}

/// Actions that commit money or compute beyond what was declared.
pub const SPEND_ACTIONS: &[&str] = &["arm_automation", "place_order", "start_campaign", "submit_job"];

/// Actions that move a strategy toward capital.
pub const PROMOTION_ACTIONS: &[&str] = &["promote_model", "promote_strategy", "raise_capital_fraction"];

/// The one-shot holdout (§12.7).
pub const SEALED_HOLDOUT_ACTIONS: &[&str] = &["evaluate_sealed_holdout", "open_vault"];

/// Durable memory later decisions will read (§13, AGENT-003).
pub const TIER3_MEMORY_ACTIONS: &[&str] = &["write_insight", "record_insight", "update_playbook"];

/// Every action that requires an approval, in one list.
///
/// This is the list the policy engine is handed at session start (§15,
/// ADR-P2-20). It lives here, with the envelopes, so there is one place that
/// decides what is gated — a second copy inside the harness would drift, and the
/// copy that drifted would be the one doing the gating.
#[must_use]
pub fn gated_actions() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = SPEND_ACTIONS
        .iter()
        .chain(PROMOTION_ACTIONS)
        .chain(SEALED_HOLDOUT_ACTIONS)
        .chain(TIER3_MEMORY_ACTIONS)
        .copied()
        .collect();
    all.sort_unstable();
    all
}

/// What the platform decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    Allowed,
    Denied,
    PendingApproval,
    Approved,
    Executed,
    Failed,
}

impl AuditVerdict {
    pub const ALL: [Self; 6] = [
        Self::Allowed,
        Self::Denied,
        Self::PendingApproval,
        Self::Approved,
        Self::Executed,
        Self::Failed,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::PendingApproval => "pending_approval",
            Self::Approved => "approved",
            Self::Executed => "executed",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == code)
    }

    /// Whether this verdict means the action did not happen. The audit trail's
    /// job is to make these visible; they are the ones that leave no other trace.
    #[must_use]
    pub fn prevented(self) -> bool {
        matches!(self, Self::Denied | Self::PendingApproval)
    }
}

/// Who asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Principal {
    pub tenant_id: String,
    pub actor_kind: String,
    pub actor_id: String,
    /// The human an agent is acting for (§8). Stamped by the API from the
    /// token's scope — never written by the agent, which is what makes the
    /// attribution non-spoofable.
    pub on_behalf_of: Option<String>,
}

/// The `pre` half: what was asked for, recorded before anyone decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditRequest {
    pub action: String,
    pub envelope: Envelope,
    pub principal: Principal,
    pub request: serde_json::Value,
}

impl AuditRequest {
    /// Classify and record an action in one step, so a caller cannot pass an
    /// envelope that disagrees with the action's name.
    #[must_use]
    pub fn new(action: impl Into<String>, principal: Principal, request: serde_json::Value) -> Self {
        let action = action.into();
        let envelope = Envelope::for_action(&action);
        Self { action, envelope, principal, request }
    }
}

/// The handle a `pre` write returns. The `post` write needs it, and the table
/// CHECKs that a `post` row has one, so a `post` without a `pre` cannot exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreAudit(Uuid);

impl PreAudit {
    #[must_use]
    pub fn id(self) -> Uuid {
        self.0
    }

    /// Crate-internal: only a successful `pre` write mints one.
    pub(crate) fn seal(id: Uuid) -> Self {
        Self(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelopes_are_exactly_the_database_check() {
        let codes: Vec<&str> = Envelope::ALL.iter().map(|e| e.as_str()).collect();
        assert_eq!(codes, vec!["none", "spend", "promotion", "sealed_holdout", "tier3_memory"]);
        for e in Envelope::ALL {
            assert_eq!(Envelope::from_code(e.as_str()), Some(e));
        }
        assert_eq!(Envelope::from_code("maybe"), None);
    }

    #[test]
    fn the_verdicts_are_exactly_the_database_check() {
        let codes: Vec<&str> = AuditVerdict::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(
            codes,
            vec!["allowed", "denied", "pending_approval", "approved", "executed", "failed"]
        );
    }

    /// AT-64's first half — exactly four things are gated, and each named action
    /// lands in exactly one of them.
    #[test]
    fn every_gated_action_is_classified_into_exactly_one_envelope() {
        assert_eq!(Envelope::GATED.len(), 4, "§15 gates four actions, no more");
        let lists = [
            (Envelope::Spend, SPEND_ACTIONS),
            (Envelope::Promotion, PROMOTION_ACTIONS),
            (Envelope::SealedHoldout, SEALED_HOLDOUT_ACTIONS),
            (Envelope::Tier3Memory, TIER3_MEMORY_ACTIONS),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for (envelope, actions) in lists {
            assert!(envelope.requires_approval());
            for action in actions {
                assert_eq!(Envelope::for_action(action), envelope, "{action}");
                assert!(!seen.contains(action), "{action} is in two envelopes");
                seen.push(action);
            }
        }
        // Everything else is ungated, which is the point of a short list.
        assert_eq!(Envelope::for_action("list_instruments"), Envelope::None);
        assert!(!Envelope::None.requires_approval());
    }

    #[test]
    fn an_audit_request_classifies_itself() {
        let p = Principal {
            tenant_id: "t".into(),
            actor_kind: "agent".into(),
            actor_id: "a".into(),
            on_behalf_of: Some("mason".into()),
        };
        let gated = AuditRequest::new("open_vault", p.clone(), serde_json::json!({}));
        assert_eq!(gated.envelope, Envelope::SealedHoldout);
        let plain = AuditRequest::new("list_lanes", p, serde_json::json!({}));
        assert_eq!(plain.envelope, Envelope::None);
    }

    /// The gated list is exactly the four envelopes' actions, and it is what the
    /// policy engine is handed. Nothing outside the four is in it.
    #[test]
    fn the_gated_list_is_the_four_envelopes() {
        let gated = gated_actions();
        assert_eq!(
            gated.len(),
            SPEND_ACTIONS.len()
                + PROMOTION_ACTIONS.len()
                + SEALED_HOLDOUT_ACTIONS.len()
                + TIER3_MEMORY_ACTIONS.len()
        );
        for action in &gated {
            assert!(
                Envelope::for_action(action).requires_approval(),
                "{action} is in the gated list but falls under no envelope"
            );
        }
        assert!(!gated.contains(&"list_instruments"));
    }

    /// A denial is the case the trail exists for: it leaves no other trace.
    #[test]
    fn the_verdicts_that_leave_no_other_trace_are_named() {
        assert!(AuditVerdict::Denied.prevented());
        assert!(AuditVerdict::PendingApproval.prevented());
        for v in [AuditVerdict::Allowed, AuditVerdict::Approved, AuditVerdict::Executed, AuditVerdict::Failed] {
            assert!(!v.prevented(), "{v:?}");
        }
    }
}

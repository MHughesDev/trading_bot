//! Proposals: the only doors into rings 1 and 2 (AGENT-013 P6, §4, §6.8, §7.5;
//! Set S task S-10.9).
//!
//! # The shape this replaces
//!
//! An agent that may write the promoted set is an agent whose *worst* step is a
//! write to the promoted set. Every other safeguard in this system — the ring
//! filter, the approval envelopes, the audit trail — narrows what such a step can
//! be; none of them makes it impossible.
//!
//! A proposal does. The agent creates a row that changes nothing. A human
//! approves it on the desk, and **the registry write happens then, as that
//! human** — not as the agent, not with the agent's token, and not at a moment
//! the agent chose. There is no path where an agent token reaches the promoted
//! set at all, which is a property a static test can check and a policy is not.
//!
//! # Creating one is cheap; it is approval that costs
//!
//! Creation is deliberately not gated: proposing costs a row. What carries the
//! envelope is the *approval*, because that is where something happens. Gating
//! creation would teach a run that asking is expensive, and a run that has
//! learned asking is expensive finds another way to get what it wants.
//!
//! # Ring 2 has preconditions, and they are not advisory
//!
//! Reaching paper needs `precapital_battery ≠ reject` and
//! `go_live_readiness ≠ no_go`, and **no proposal may be created while the kill
//! switch is tripped**. The last one is the one worth stating: a tripped switch
//! means the platform has decided something is wrong, and a proposal created
//! during it is a proposal that will be sitting in the queue looking reasonable
//! when somebody resets the switch an hour later.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::audit::Envelope;

/// What a proposal asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    /// Register a strategy in the promoted set. Ring 1.
    PromoteStrategy,
    /// Change a live risk limit. Ring 1 by target, `Promotion` by envelope —
    /// widening a limit moves a strategy toward capital as surely as promoting
    /// one does.
    ChangeLimits,
    /// Arm an automation so it trades on paper. Ring 2.
    DeployPaper,
}

impl ProposalKind {
    /// Whether an approval of this kind needs more than [`Proposal::subject_id`].
    ///
    /// `deploy_paper` does not: arming an automation is "this row, on", and the
    /// row is the target. `change_limits` does, because a limit is eight fields
    /// and a name describes none of them. `promote_strategy` does **not**
    /// today, and that is not a claim that promotion needs nothing - it is that
    /// the promoted set has no writer and therefore no shape to validate
    /// against, which is the decision S-10.9 is still waiting on. When it has
    /// one, this returns `true` for it and `propose` starts demanding it.
    #[must_use]
    pub fn carries_a_payload(self) -> bool {
        matches!(self, ProposalKind::ChangeLimits)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalKind::PromoteStrategy => "promote_strategy",
            ProposalKind::ChangeLimits => "change_limits",
            ProposalKind::DeployPaper => "deploy_paper",
        }
    }

    /// The ring this proposal reaches into.
    #[must_use]
    pub fn ring(self) -> u8 {
        match self {
            ProposalKind::PromoteStrategy | ProposalKind::ChangeLimits => 1,
            ProposalKind::DeployPaper => 2,
        }
    }

    /// The approval envelope the *decision* carries.
    ///
    /// Derived from the kind rather than supplied, for the reason every derived
    /// field in this crate is derived: a caller that could name its own envelope
    /// would name `None`.
    #[must_use]
    pub fn envelope(self) -> Envelope {
        match self {
            // Moving a strategy toward capital, which is what `Promotion` is
            // defined as — and widening a limit does it as surely as promoting.
            ProposalKind::PromoteStrategy | ProposalKind::ChangeLimits => Envelope::Promotion,
            // Paper trading spends. The envelope is about the money, and paper
            // money is the rehearsal for the other kind.
            ProposalKind::DeployPaper => Envelope::Spend,
        }
    }

    pub const ALL: &'static [ProposalKind] = &[
        ProposalKind::PromoteStrategy,
        ProposalKind::ChangeLimits,
        ProposalKind::DeployPaper,
    ];
}

/// What the platform knew when the proposal was created.
///
/// Passed in rather than read here, because this crate performs no I/O — and
/// because the *caller* is the one that can prove these came from the platform
/// rather than from the run's prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preconditions {
    /// `precapital_battery` returned `reject`.
    pub battery_rejected: bool,
    /// `go_live_readiness` returned `no_go`.
    pub readiness_no_go: bool,
    /// The kill switch is tripped.
    pub kill_switch_tripped: bool,
}

/// Why a proposal was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProposalError {
    #[error(
        "the kill switch is tripped. Nothing is proposed while the platform has decided \
         something is wrong — a proposal created now would be sitting in the queue looking \
         reasonable when somebody resets the switch an hour later."
    )]
    KillSwitchTripped,
    #[error("`precapital_battery` rejected this candidate; ring 2 is not reachable from a reject")]
    BatteryRejected,
    #[error("`go_live_readiness` says no-go; ring 2 is not reachable from a no-go")]
    ReadinessNoGo,
    #[error("a proposal must cite the dossier the decision will be made from")]
    NoDossier,
    #[error("a proposal must name the row the approval will act on. The subject is what the human reads; the id is what the platform writes to, and resolving the one into the other at approval time would put a guess at the moment the platform acts on somebody's authority.")]
    NoTarget,
    #[error("{0}")]
    BadPayload(String),
}

/// A request for a human decision (§7.5).
///
/// Changing nothing is the whole design. The row exists, the desk renders it, a
/// human decides — and the registry write happens at *that* moment, as that
/// human.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal_id: Uuid,
    pub run_id: Uuid,
    pub kind: ProposalKind,
    /// Derived from the kind. Recorded so a reader does not have to re-derive it,
    /// and asserted equal to the derivation by `the_stored_ring_is_the_derived_one`.
    pub ring: u8,
    pub envelope: Envelope,
    /// What the decision is made from. A proposal with no dossier asks a human to
    /// decide from the proposal itself, which is the agent's own summary of its
    /// own work.
    pub dossier_ref: String,
    /// What is being promoted, armed or changed, as the human reads it.
    pub subject: String,
    /// The row the approval acts on (migration 0131).
    ///
    /// Separate from `subject` because they answer different questions. A human
    /// approving `deploy_paper` for `"ema_cross_v3"` needs to read that name;
    /// the platform performing the approval needs an id. Resolving the name to a
    /// row at approval time would put a guess at the one moment the platform
    /// acts on somebody's authority — which is the free-form-string failure this
    /// repository keeps paying for, at the worst possible site.
    pub subject_id: Uuid,
    /// One line, in the run's words, on why.
    pub rationale: String,
    /// What the approval would write (migration `0137`).
    ///
    /// Per kind, because the shape is: `deploy_paper` needs nothing beyond
    /// [`Self::subject_id`] — arming a row is "this row, on" — and
    /// `change_limits` needs the whole limit, which has eight fields and none of
    /// them derivable from a name.
    ///
    /// Shape-checked here and **type**-checked by the caller that owns the
    /// target type, at creation. The alternative is letting the approval invent
    /// what the proposal did not carry, which is the failure ADR-S10-09 records
    /// for `subject_id`, sited at the one moment the platform acts on somebody
    /// else's authority.
    pub payload: Value,
}

/// Create one, or say why not (§6.8).
///
/// # Errors
/// [`ProposalError`] — the kill switch, the ring-2 preconditions, or a missing
/// dossier.
#[allow(clippy::too_many_arguments)]
pub fn propose(
    run_id: Uuid,
    kind: ProposalKind,
    dossier_ref: &str,
    subject: &str,
    subject_id: Uuid,
    rationale: &str,
    payload: Value,
    pre: Preconditions,
) -> Result<Proposal, ProposalError> {
    // The kill switch first, and for every kind. A tripped switch is the
    // platform saying something is wrong *now*; which ring the proposal reaches
    // is a question about later.
    if pre.kill_switch_tripped {
        return Err(ProposalError::KillSwitchTripped);
    }
    if kind.ring() >= 2 {
        if pre.battery_rejected {
            return Err(ProposalError::BatteryRejected);
        }
        if pre.readiness_no_go {
            return Err(ProposalError::ReadinessNoGo);
        }
    }
    if dossier_ref.trim().is_empty() {
        return Err(ProposalError::NoDossier);
    }
    // The nil UUID is what an absent id deserialises to if anything ever
    // defaults it, and it names no row. Refused here rather than at the
    // approval, which is where it would be discovered by a human who had
    // already decided.
    if subject_id.is_nil() {
        return Err(ProposalError::NoTarget);
    }
    // Shape only. This crate does not know what a risk limit is, and a second
    // opinion about its fields here would be a second standard - the caller
    // that owns `ControlLimit` checks it against the real type. What is checked
    // here is the pairing of kind and payload, which is this type's business.
    if !payload.is_object() {
        return Err(ProposalError::BadPayload(
            "a payload is an object; `{}` is how a kind says it carries nothing".into(),
        ));
    }
    let empty = payload.as_object().is_none_or(serde_json::Map::is_empty);
    if kind.carries_a_payload() && empty {
        return Err(ProposalError::BadPayload(format!(
            "`{}` changes something a name does not describe, so the proposal has to carry \
             what would be written. Approving one that does not would mean inventing it.",
            kind.as_str()
        )));
    }
    if !kind.carries_a_payload() && !empty {
        // Not ignored. A payload nothing reads is a caller believing it said
        // something, and the first person to find out would be whoever wondered
        // why the approval did not do what the payload described.
        return Err(ProposalError::BadPayload(format!(
            "`{}` acts on `subject_id` and reads no payload; remove it rather than sending a \
             value nothing will apply",
            kind.as_str()
        )));
    }
    Ok(Proposal {
        proposal_id: Uuid::new_v4(),
        run_id,
        kind,
        ring: kind.ring(),
        envelope: kind.envelope(),
        dossier_ref: dossier_ref.trim().to_string(),
        subject: subject.trim().to_string(),
        subject_id,
        rationale: rationale.trim().to_string(),
        payload,
    })
}

/// What a human did with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Pending,
    Approved,
    Rejected,
}

impl Decision {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Pending => "pending",
            Decision::Approved => "approved",
            Decision::Rejected => "rejected",
        }
    }

    /// Whether the registry write may happen.
    ///
    /// Only on an explicit approval. `Pending` is not "not yet rejected" — it is
    /// a decision nobody has made, and treating it as permission is how an
    /// approval queue becomes a delay.
    #[must_use]
    pub fn permits_write(self) -> bool {
        self == Decision::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear() -> Preconditions {
        Preconditions::default()
    }

    /// The payload a kind requires, so a test about something *else* does not
    /// have to know the rule.
    fn payload_for(kind: ProposalKind) -> Value {
        if kind.carries_a_payload() {
            serde_json::json!({ "metric": "max_drawdown", "upper": 0.2 })
        } else {
            serde_json::json!({})
        }
    }

    #[test]
    fn a_proposal_changes_nothing_and_cites_what_the_decision_rests_on() {
        let p = propose(
            Uuid::new_v4(),
            ProposalKind::PromoteStrategy,
            "dossier_41",
            "ema_cross_v3",
            Uuid::new_v4(),
            "survived the battery on two venues",
            payload_for(ProposalKind::PromoteStrategy),
            clear(),
        )
        .expect("clear preconditions");
        assert_eq!(p.ring, 1);
        assert_eq!(p.envelope, Envelope::Promotion);
        assert_eq!(p.dossier_ref, "dossier_41");
    }

    /// A proposal with no dossier asks a human to decide from the agent's own
    /// summary of the agent's own work.
    #[test]
    fn a_proposal_without_a_dossier_is_refused() {
        let err = propose(
            Uuid::new_v4(),
            ProposalKind::PromoteStrategy,
            "   ",
            "ema_cross_v3",
            Uuid::new_v4(),
            "it looks good",
            payload_for(ProposalKind::PromoteStrategy),
            clear(),
        )
        .expect_err("refused");
        assert_eq!(err, ProposalError::NoDossier);
    }

    /// Nothing is proposed while the kill switch is tripped — for **any** kind.
    #[test]
    fn a_tripped_kill_switch_stops_every_proposal() {
        let pre = Preconditions {
            kill_switch_tripped: true,
            ..clear()
        };
        for kind in ProposalKind::ALL {
            let err = propose(
                Uuid::new_v4(),
                *kind,
                "d_1",
                "s",
                Uuid::new_v4(),
                "r",
                payload_for(*kind),
                pre,
            )
            .expect_err("refused");
            assert_eq!(err, ProposalError::KillSwitchTripped, "{kind:?}");
        }
        let text = ProposalError::KillSwitchTripped.to_string();
        assert!(
            text.contains("resets the switch an hour later"),
            "the reason is the point: {text}"
        );
    }

    /// Ring 2 needs the battery and the readiness check.
    #[test]
    fn paper_is_not_reachable_from_a_reject_or_a_no_go() {
        let rejected = Preconditions {
            battery_rejected: true,
            ..clear()
        };
        assert_eq!(
            propose(
                Uuid::new_v4(),
                ProposalKind::DeployPaper,
                "d",
                "s",
                Uuid::new_v4(),
                "r",
                payload_for(ProposalKind::DeployPaper),
                rejected
            ),
            Err(ProposalError::BatteryRejected)
        );
        let no_go = Preconditions {
            readiness_no_go: true,
            ..clear()
        };
        assert_eq!(
            propose(
                Uuid::new_v4(),
                ProposalKind::DeployPaper,
                "d",
                "s",
                Uuid::new_v4(),
                "r",
                payload_for(ProposalKind::DeployPaper),
                no_go
            ),
            Err(ProposalError::ReadinessNoGo)
        );
    }

    /// The ring-2 preconditions do not apply to ring 1.
    ///
    /// A candidate the battery rejected may still be worth *registering* — the
    /// registry is where a strategy lives, not where it trades, and refusing the
    /// write would make the platform forget the thing it just refuted.
    #[test]
    fn a_rejected_candidate_may_still_be_registered() {
        let rejected = Preconditions {
            battery_rejected: true,
            ..clear()
        };
        propose(
            Uuid::new_v4(),
            ProposalKind::PromoteStrategy,
            "d",
            "s",
            Uuid::new_v4(),
            "r",
            payload_for(ProposalKind::PromoteStrategy),
            rejected,
        )
        .expect("ring 1 does not ask the battery");
    }

    /// A proposal with no target is refused where it is cheap, not at the
    /// approval where a human has already decided.
    #[test]
    fn a_proposal_with_no_target_is_refused() {
        let err = propose(
            Uuid::new_v4(),
            ProposalKind::DeployPaper,
            "dossier_1",
            "ema_cross_v3",
            Uuid::nil(),
            "it survived",
            payload_for(ProposalKind::DeployPaper),
            clear(),
        )
        .expect_err("the nil uuid names no row");
        assert_eq!(err, ProposalError::NoTarget);
        assert!(
            format!("{err}").contains("what the human reads"),
            "the refusal must say why the subject is not enough"
        );
    }

    /// The envelope is derived, never supplied.
    #[test]
    fn each_kind_carries_the_envelope_its_target_implies() {
        assert_eq!(
            ProposalKind::PromoteStrategy.envelope(),
            Envelope::Promotion
        );
        assert_eq!(
            ProposalKind::ChangeLimits.envelope(),
            Envelope::Promotion,
            "widening a limit moves a strategy toward capital as surely as promoting one"
        );
        assert_eq!(ProposalKind::DeployPaper.envelope(), Envelope::Spend);
        for kind in ProposalKind::ALL {
            assert!(
                Envelope::GATED.contains(&kind.envelope()),
                "{kind:?} must fall under an approval envelope"
            );
        }
    }

    /// The stored ring is the derived one.
    #[test]
    fn the_stored_ring_is_the_derived_one() {
        for kind in ProposalKind::ALL {
            let p = propose(
                Uuid::new_v4(),
                *kind,
                "d",
                "s",
                Uuid::new_v4(),
                "r",
                payload_for(*kind),
                clear(),
            )
            .unwrap();
            assert_eq!(p.ring, kind.ring());
            assert_eq!(p.envelope, kind.envelope());
        }
    }

    /// Pending is not permission.
    ///
    /// "Not yet rejected" is how an approval queue becomes a delay, and a delay
    /// is something a busy person clears.
    #[test]
    fn only_an_explicit_approval_permits_the_write() {
        assert!(Decision::Approved.permits_write());
        assert!(!Decision::Pending.permits_write());
        assert!(!Decision::Rejected.permits_write());
    }

    /// A kind that changes something a name does not describe has to carry what
    /// would be written.
    ///
    /// The alternative is an approval that invents the fields the proposal did
    /// not carry — at the one moment the platform acts on somebody else's
    /// authority, which is exactly where ADR-S10-09 says a guess must not be.
    #[test]
    fn a_kind_that_writes_more_than_a_row_id_must_say_what() {
        let err = propose(
            Uuid::new_v4(),
            ProposalKind::ChangeLimits,
            "d_1",
            "max_drawdown on ema_cross_v3",
            Uuid::new_v4(),
            "the paper session ran tighter than the backtest",
            serde_json::json!({}),
            clear(),
        )
        .expect_err("a limit change with no limit");
        assert!(
            matches!(err, ProposalError::BadPayload(ref m) if m.contains("inventing")),
            "{err:?}"
        );
    }

    /// And a payload on a kind that reads none is refused rather than ignored.
    ///
    /// Ignoring it would leave a caller believing it had said something, and the
    /// first person to find out would be whoever wondered why the approval did
    /// not do what the payload described.
    #[test]
    fn a_payload_nothing_will_read_is_refused_rather_than_dropped() {
        for kind in ProposalKind::ALL.iter().filter(|k| !k.carries_a_payload()) {
            let err = propose(
                Uuid::new_v4(),
                *kind,
                "d_1",
                "s",
                Uuid::new_v4(),
                "r",
                serde_json::json!({ "upper": 0.9 }),
                clear(),
            )
            .expect_err("a payload nothing applies");
            assert!(
                matches!(err, ProposalError::BadPayload(ref m) if m.contains("remove it")),
                "{kind:?}: {err:?}"
            );
        }
    }

    /// `{}` is how a kind says it carries nothing, and a non-object is not that.
    #[test]
    fn a_payload_that_is_not_an_object_is_refused() {
        for bad in [
            serde_json::json!(null),
            serde_json::json!([1]),
            serde_json::json!("limit"),
        ] {
            let err = propose(
                Uuid::new_v4(),
                ProposalKind::DeployPaper,
                "d_1",
                "s",
                Uuid::new_v4(),
                "r",
                bad.clone(),
                clear(),
            )
            .expect_err("not an object");
            assert!(
                matches!(err, ProposalError::BadPayload(_)),
                "{bad}: {err:?}"
            );
        }
    }

    /// The payload survives onto the proposal unchanged. It is what the approval
    /// writes, so a proposal that altered it would be approving one thing and
    /// performing another.
    #[test]
    fn the_payload_reaches_the_proposal_unchanged() {
        let limit = serde_json::json!({
            "metric": "max_drawdown",
            "family": "behaviour",
            "severity": "breach",
            "version": 3,
            "upper": 0.2,
            "method": "paper_ewma_3sigma",
            "declared_from": "bt_991"
        });
        let p = propose(
            Uuid::new_v4(),
            ProposalKind::ChangeLimits,
            "d_1",
            "max_drawdown on ema_cross_v3",
            Uuid::new_v4(),
            "the paper session ran tighter",
            limit.clone(),
            clear(),
        )
        .expect("a complete limit change");
        assert_eq!(p.payload, limit);
        assert_eq!(p.ring, 1);
        assert_eq!(p.envelope, Envelope::Promotion);
    }

    /// `promote_strategy` carries nothing **today**, and the reason is not that
    /// promotion needs nothing — it is that the promoted set has no writer, so
    /// there is no shape to validate against. This pins the answer so that
    /// building one is a deliberate change here rather than a surprise.
    #[test]
    fn only_a_limit_change_carries_a_payload_today() {
        let carrying: Vec<&str> = ProposalKind::ALL
            .iter()
            .filter(|k| k.carries_a_payload())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            carrying,
            vec!["change_limits"],
            "the set changed; `perform` in api::routes::proposals must gain the writer"
        );
    }
}

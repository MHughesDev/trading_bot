//! The training job state machine (SPEC §9). Mirrors
//! `mlops.trial_transition_legal` in migration 0043; `sql_and_rust_agree` pins them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialState {
    Registered,
    Queued,
    Rejected,
    Deduplicated,
    Provision,
    Running,
    Evaluate,
    Paused,
    Preempted,
    Recovering,
    Gated,
    /// Evaluated but not submitted to the gate stack (a Study member, say).
    Completed,
    CompletedPass,
    CompletedFail,
    Failed,
}

pub const ALL_STATES: &[TrialState] = &[
    TrialState::Registered,
    TrialState::Queued,
    TrialState::Rejected,
    TrialState::Deduplicated,
    TrialState::Provision,
    TrialState::Running,
    TrialState::Evaluate,
    TrialState::Paused,
    TrialState::Preempted,
    TrialState::Recovering,
    TrialState::Gated,
    TrialState::Completed,
    TrialState::CompletedPass,
    TrialState::CompletedFail,
    TrialState::Failed,
];

impl TrialState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Queued => "queued",
            Self::Rejected => "rejected",
            Self::Deduplicated => "deduplicated",
            Self::Provision => "provision",
            Self::Running => "running",
            Self::Evaluate => "evaluate",
            Self::Paused => "paused",
            Self::Preempted => "preempted",
            Self::Recovering => "recovering",
            Self::Gated => "gated",
            Self::Completed => "completed",
            Self::CompletedPass => "completed_pass",
            Self::CompletedFail => "completed_fail",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        ALL_STATES.iter().copied().find(|st| st.as_str() == s)
    }

    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Deduplicated | Self::Completed | Self::CompletedPass | Self::CompletedFail | Self::Failed
        )
    }

    #[must_use]
    pub fn can_transition_to(self, to: Self) -> bool {
        use TrialState::{
            Completed, CompletedFail, CompletedPass, Deduplicated, Evaluate, Failed, Gated, Paused, Preempted, Provision,
            Queued, Recovering, Registered, Rejected, Running,
        };
        // One arm per source state, so the compiler proves every state is covered.
        match self {
            Registered => matches!(to, Queued | Rejected | Deduplicated | Running | Failed),
            Queued => matches!(to, Provision | Running | Failed),
            Provision | Recovering => matches!(to, Running | Failed),
            Running => matches!(to, Evaluate | Paused | Preempted | Failed | Completed),
            Paused => matches!(to, Queued | Failed),
            Preempted => matches!(to, Recovering | Failed),
            Evaluate => matches!(to, Gated | Completed | CompletedFail | Failed),
            Gated => matches!(to, CompletedPass | CompletedFail),
            Rejected | Deduplicated | Completed | CompletedPass | CompletedFail | Failed => false,
        }
    }
}

/// Why a trial stopped producing observations (SPEC §4.2·5). An ASHA-killed run is
/// right-censored, not failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Censoring {
    None,
    RightAsha,
    RightBudget,
    RightPreempt,
    RightCancel,
    Failed,
}

impl Censoring {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RightAsha => "right_asha",
            Self::RightBudget => "right_budget",
            Self::RightPreempt => "right_preempt",
            Self::RightCancel => "right_cancel",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::None, Self::RightAsha, Self::RightBudget, Self::RightPreempt, Self::RightCancel, Self::Failed]
            .into_iter()
            .find(|c| c.as_str() == s)
    }

    #[must_use]
    pub fn is_right_censored(self) -> bool {
        matches!(self, Self::RightAsha | Self::RightBudget | Self::RightPreempt | Self::RightCancel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalReason {
    Oom,
    NanDivergence,
    DataError,
    Timeout,
    LeakageDetected,
    BudgetExceeded,
    Cancelled,
    DependencyFailure,
    AshaStopped,
    IntegrityRejected,
    PreemptedAbandoned,
    GateFailed,
}

impl TerminalReason {
    /// The closed set of §9 terminal reasons, in spec order.
    ///
    /// This is the list every other surface is checked against: the job
    /// service's failures, the trainer sidecar's `terminal` field, and the
    /// database CHECK. A reason that is not here does not exist.
    pub const ALL: [Self; 12] = [
        Self::Oom,
        Self::NanDivergence,
        Self::DataError,
        Self::Timeout,
        Self::LeakageDetected,
        Self::BudgetExceeded,
        Self::Cancelled,
        Self::DependencyFailure,
        Self::AshaStopped,
        Self::IntegrityRejected,
        Self::PreemptedAbandoned,
        Self::GateFailed,
    ];

    /// Parse a wire code produced by [`Self::as_str`].
    ///
    /// Returns `None` for anything else rather than falling back to a reason:
    /// a caller that sent an unrecognised code has not told us how the trial
    /// ended, and the caller decides what to record about that. Nothing here
    /// matches on substrings — `"nan_divergence"` parses, `"loss was nan"` does
    /// not, and that is the point (ADR-P2-30).
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == code)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oom => "oom",
            Self::NanDivergence => "nan_divergence",
            Self::DataError => "data_error",
            Self::Timeout => "timeout",
            Self::LeakageDetected => "leakage_detected",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Cancelled => "cancelled",
            Self::DependencyFailure => "dependency_failure",
            Self::AshaStopped => "asha_stopped",
            Self::IntegrityRejected => "integrity_rejected",
            Self::PreemptedAbandoned => "preempted_abandoned",
            Self::GateFailed => "gate_failed",
        }
    }

    /// The censoring this reason implies. Enforced by CHECK in the database too.
    #[must_use]
    pub fn censoring(self) -> Censoring {
        match self {
            Self::AshaStopped => Censoring::RightAsha,
            Self::BudgetExceeded => Censoring::RightBudget,
            Self::Cancelled => Censoring::RightCancel,
            Self::PreemptedAbandoned => Censoring::RightPreempt,
            Self::GateFailed => Censoring::None,
            Self::IntegrityRejected | Self::Oom | Self::NanDivergence | Self::DataError | Self::Timeout
            | Self::LeakageDetected | Self::DependencyFailure => Censoring::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AT-60 — the terminal-reason set is exactly SPEC §9's, and its codes are
    /// the wire contract with the job service, the trainer sidecar and the
    /// database CHECK. Any drift in either direction fails here.
    #[test]
    fn terminal_reasons_are_exactly_the_specs_closed_set() {
        let codes: Vec<&str> = TerminalReason::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "oom",
                "nan_divergence",
                "data_error",
                "timeout",
                "leakage_detected",
                "budget_exceeded",
                "cancelled",
                "dependency_failure",
                "asha_stopped",
                "integrity_rejected",
                "preempted_abandoned",
                "gate_failed",
            ]
        );
    }

    /// AT-60 — parsing is exact. A message that merely *contains* a reason's
    /// name is not that reason; this is the behaviour the substring sniffers
    /// used to have and the reason a reworded error changed a trial's
    /// censoring.
    #[test]
    fn codes_round_trip_and_nothing_else_parses() {
        for r in TerminalReason::ALL {
            assert_eq!(TerminalReason::from_code(r.as_str()), Some(r));
        }
        for junk in ["", "nan", "loss became nan", "NAN_DIVERGENCE", "data", "oom_killer"] {
            assert_eq!(TerminalReason::from_code(junk), None, "{junk:?} must not parse");
        }
    }

    /// AT-60 — `asha_stopped` settles `right_asha`, never `failed`. A trial the
    /// scheduler stopped early is a censored observation, and counting it as a
    /// failure is how a search that works starts looking like one that does not.
    #[test]
    fn every_reason_has_the_censoring_spec_9_gives_it() {
        use Censoring::{Failed, None as NoCensoring, RightAsha, RightBudget, RightCancel, RightPreempt};
        for (reason, expected) in [
            (TerminalReason::AshaStopped, RightAsha),
            (TerminalReason::BudgetExceeded, RightBudget),
            (TerminalReason::Cancelled, RightCancel),
            (TerminalReason::PreemptedAbandoned, RightPreempt),
            (TerminalReason::GateFailed, NoCensoring),
            (TerminalReason::Oom, Failed),
            (TerminalReason::NanDivergence, Failed),
            (TerminalReason::DataError, Failed),
            (TerminalReason::Timeout, Failed),
            (TerminalReason::LeakageDetected, Failed),
            (TerminalReason::DependencyFailure, Failed),
            (TerminalReason::IntegrityRejected, Failed),
        ] {
            assert_eq!(reason.censoring(), expected, "{reason:?}");
        }
        assert_ne!(TerminalReason::AshaStopped.censoring(), Failed);
    }

    #[test]
    fn terminal_states_have_no_exits() {
        for s in ALL_STATES {
            if s.is_terminal() {
                for t in ALL_STATES {
                    assert!(!s.can_transition_to(*t), "{s:?} is terminal but can go to {t:?}");
                }
            }
        }
    }

    #[test]
    fn preemption_resumes_without_a_new_trial() {
        use TrialState::*;
        let path = [Registered, Queued, Provision, Running, Preempted, Recovering, Running, Evaluate, Gated, CompletedPass];
        for w in path.windows(2) {
            assert!(w[0].can_transition_to(w[1]), "{:?} -> {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn cannot_skip_the_gate_to_pass() {
        assert!(!TrialState::Evaluate.can_transition_to(TrialState::CompletedPass));
        assert!(!TrialState::Running.can_transition_to(TrialState::CompletedPass));
    }

    #[test]
    fn round_trip_names() {
        for s in ALL_STATES {
            assert_eq!(TrialState::parse(s.as_str()), Some(*s));
        }
    }
}

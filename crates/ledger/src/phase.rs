//! The campaign phase machine (SPEC §10, checklist 2.1, ADR-P2-04).
//!
//! A campaign has no status column. Its state is a **fold over
//! `mlops.campaign_event`**, which is append-only and immutable like every other
//! ledger table. That is the whole durability design: resumption is replay, and
//! there is nothing else to resume, so a driver killed mid-phase and restarted
//! computes the same state it had — not a state someone remembered to write down
//! before the process died.
//!
//! The alternative, a workflow engine holding the state, was rejected in
//! ADR-P2-04: Temporal is not in this stack, the phases are coarse (minutes to
//! hours, not milliseconds), and a task queue with explicit checkpoints is
//! sufficient at that granularity. What a workflow engine would have bought —
//! exactly-once side effects — comes from the job service instead: each phase's
//! work is a **child job** keyed by `(campaign job, phase, seq)`, and the job
//! service already refuses a duplicate child at that key.
//!
//! ## Why the transitions are checked here *and* in the database
//!
//! `fold` refuses an illegal sequence, and so does the trigger added by
//! migration 0052. Neither is redundant: the fold is what a driver consults
//! before acting, the trigger is what stops a second writer — a script, a
//! future service, a person with psql — from appending a `gate` after a
//! `halted` and making every later fold disagree with every earlier one.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::LedgerError;

/// A campaign phase (§10). These are exactly the values
/// `mlops.campaign_event.state` accepts; the CHECK and this enum are the same
/// closed set stated twice, and `phases_match_the_database_check` fails if they
/// drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignPhase {
    Define,
    Baseline,
    Diagnose,
    Hypothesize,
    Experiment,
    Compare,
    Gate,
    Prune,
    Reallocate,
    Converged,
    BudgetExhausted,
    DiminishingReturns,
    Halted,
}

use CampaignPhase::{
    Baseline, BudgetExhausted, Compare, Converged, Define, Diagnose, DiminishingReturns,
    Experiment, Gate, Halted, Hypothesize, Prune, Reallocate,
};

impl CampaignPhase {
    /// Every phase, in §10 order.
    pub const ALL: [Self; 13] = [
        Define,
        Baseline,
        Diagnose,
        Hypothesize,
        Experiment,
        Compare,
        Gate,
        Prune,
        Reallocate,
        Converged,
        BudgetExhausted,
        DiminishingReturns,
        Halted,
    ];

    /// The four endings. A campaign in one of these is over: nothing may be
    /// appended after it, which is what makes "spends no second trial after
    /// termination" a property of the log rather than of the driver's care.
    pub const TERMINAL: [Self; 4] = [Converged, BudgetExhausted, DiminishingReturns, Halted];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Define => "define",
            Baseline => "baseline",
            Diagnose => "diagnose",
            Hypothesize => "hypothesize",
            Experiment => "experiment",
            Compare => "compare",
            Gate => "gate",
            Prune => "prune",
            Reallocate => "reallocate",
            Converged => "converged",
            BudgetExhausted => "budget_exhausted",
            DiminishingReturns => "diminishing_returns",
            Halted => "halted",
        }
    }

    /// Exact parse of a stored state. `None` for anything else — a row whose
    /// state this code does not know is not silently folded into a phase it
    /// might not be.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == code)
    }

    #[must_use]
    pub fn is_terminal(self) -> bool {
        Self::TERMINAL.contains(&self)
    }

    /// Whether this phase's work is dispatched as a child job. The others are
    /// bookkeeping the driver does itself and are recorded, not executed.
    #[must_use]
    pub fn has_child_job(self) -> bool {
        matches!(self, Baseline | Diagnose | Experiment | Compare | Gate)
    }

    /// The phases that may follow this one.
    ///
    /// Every non-terminal phase may also end: a budget runs out, a human halts
    /// it, the posterior stops moving. Those are in [`Self::TERMINAL`] and are
    /// allowed from anywhere, so they are not repeated here.
    #[must_use]
    pub fn successors(self) -> &'static [Self] {
        match self {
            Define => &[Baseline],
            Baseline => &[Diagnose],
            // The first cycle arrives at HYPOTHESIZE from DIAGNOSE and every
            // later one from REALLOCATE; the loop closes at the same place.
            Diagnose | Reallocate => &[Hypothesize],
            // A cycle that produced nothing worth comparing still has to say so;
            // `hypothesize` twice in a row is the agent revising its proposal.
            Hypothesize => &[Experiment, Hypothesize],
            Experiment => &[Compare],
            Compare => &[Gate],
            Gate => &[Prune, Reallocate],
            Prune => &[Reallocate, Hypothesize],
            Converged | BudgetExhausted | DiminishingReturns | Halted => &[],
        }
    }

    /// Whether `next` may follow `self`.
    #[must_use]
    pub fn may_precede(self, next: Self) -> bool {
        if self.is_terminal() {
            return false;
        }
        next.is_terminal() || self.successors().contains(&next)
    }
}

/// One appended fact about a campaign.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CampaignEvent {
    pub phase: CampaignPhase,
    /// Whatever the phase recorded: the child job's id, the comparison verdict,
    /// the reason a human halted it. Data for readers, never a second state.
    #[serde(default)]
    pub detail: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

impl CampaignEvent {
    #[must_use]
    pub fn new(phase: CampaignPhase, detail: serde_json::Value) -> Self {
        Self { phase, detail, occurred_at: Utc::now() }
    }

    /// The child job this event dispatched, if it recorded one.
    #[must_use]
    pub fn child_job_id(&self) -> Option<&str> {
        self.detail.get("job_id").and_then(serde_json::Value::as_str)
    }
}

/// A campaign's state: everything derived, nothing stored.
#[derive(Clone, Debug, PartialEq)]
pub struct CampaignState {
    /// The phase the last event put it in.
    pub phase: CampaignPhase,
    /// How many events the fold consumed. This is the `seq` the next child job
    /// is keyed by, so two drivers folding the same log key the same child.
    pub seq: i64,
    /// Completed `hypothesize → … → reallocate` loops.
    pub cycles: i64,
    /// `Some` once the campaign has ended.
    pub terminal: Option<CampaignPhase>,
    /// Child jobs dispatched, oldest first, as `(phase, seq, job_id)`.
    pub children: Vec<(CampaignPhase, i64, String)>,
}

impl CampaignState {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// The phase to run next, or `None` if the campaign is over.
    ///
    /// Ambiguity is resolved by taking the first successor, which is the §10
    /// order; the driver overrides it when a phase's result says to (a gate that
    /// failed prunes, one that passed reallocates).
    #[must_use]
    pub fn next_phase(&self) -> Option<CampaignPhase> {
        if self.is_terminal() {
            return None;
        }
        self.phase.successors().first().copied()
    }

    /// The idempotency key for the work of `phase` at the current `seq`
    /// (ADR-P2-04). Stable across restarts because both inputs are folds of the
    /// same immutable log, which is what makes a resumed driver reuse the child
    /// job it already created instead of starting a second one.
    #[must_use]
    pub fn child_key(&self, campaign_id: Uuid, phase: CampaignPhase) -> String {
        let mut h = Sha256::new();
        h.update(campaign_id.as_bytes());
        h.update(b"\x1f");
        h.update(phase.as_str().as_bytes());
        h.update(b"\x1f");
        h.update(self.seq.to_be_bytes());
        format!("sha256:{}", hex::encode(h.finalize()))
    }
}

/// Fold a campaign's events into its state.
///
/// # Errors
/// An empty log (a campaign always opens with `define`), a log that does not
/// open with `define`, an illegal transition, or anything appended after a
/// terminal phase. All four are corruption of an append-only table, so this
/// reports them rather than repairing them: a fold that quietly skipped a bad
/// row would make two readers of the same log disagree.
pub fn fold(events: &[CampaignEvent]) -> Result<CampaignState, LedgerError> {
    let Some(first) = events.first() else {
        return Err(LedgerError::Invalid(
            "a campaign has no events; DEFINE writes the first one and nothing else can".into(),
        ));
    };
    if first.phase != Define {
        return Err(LedgerError::Invalid(format!(
            "a campaign's first event must be `define`, not `{}`",
            first.phase.as_str()
        )));
    }

    let mut state = CampaignState {
        phase: Define,
        seq: 1,
        cycles: 0,
        terminal: None,
        children: Vec::new(),
    };
    if let Some(id) = first.child_job_id() {
        state.children.push((Define, 0, id.to_string()));
    }

    for (i, ev) in events.iter().enumerate().skip(1) {
        let seq = i64::try_from(i).unwrap_or(i64::MAX);
        if let Some(end) = state.terminal {
            return Err(LedgerError::Invalid(format!(
                "campaign ended in `{}`; `{}` was appended after it",
                end.as_str(),
                ev.phase.as_str()
            )));
        }
        if !state.phase.may_precede(ev.phase) {
            return Err(LedgerError::Invalid(format!(
                "`{}` cannot follow `{}` (SPEC §10)",
                ev.phase.as_str(),
                state.phase.as_str()
            )));
        }
        if ev.phase == Reallocate {
            state.cycles += 1;
        }
        if ev.phase.is_terminal() {
            state.terminal = Some(ev.phase);
        }
        if let Some(id) = ev.child_job_id() {
            state.children.push((ev.phase, seq, id.to_string()));
        }
        state.phase = ev.phase;
        state.seq = seq + 1;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(phase: CampaignPhase) -> CampaignEvent {
        CampaignEvent::new(phase, serde_json::json!({}))
    }

    fn with_job(phase: CampaignPhase, job: &str) -> CampaignEvent {
        CampaignEvent::new(phase, serde_json::json!({ "job_id": job }))
    }

    #[test]
    fn phases_match_the_database_check() {
        // Migration 0043's CHECK, verbatim and in order.
        let codes: Vec<&str> = CampaignPhase::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "define",
                "baseline",
                "diagnose",
                "hypothesize",
                "experiment",
                "compare",
                "gate",
                "prune",
                "reallocate",
                "converged",
                "budget_exhausted",
                "diminishing_returns",
                "halted",
            ]
        );
        for p in CampaignPhase::ALL {
            assert_eq!(CampaignPhase::from_code(p.as_str()), Some(p));
        }
        assert_eq!(CampaignPhase::from_code("running"), None);
    }

    #[test]
    fn a_campaign_opens_with_define_and_nothing_else() {
        assert!(fold(&[]).is_err());
        assert!(fold(&[ev(Baseline)]).is_err());
        assert_eq!(fold(&[ev(Define)]).unwrap().phase, Define);
    }

    #[test]
    fn the_fold_is_the_state() {
        let log = vec![
            ev(Define),
            with_job(Baseline, "job_1"),
            with_job(Diagnose, "job_2"),
            ev(Hypothesize),
            with_job(Experiment, "job_3"),
            with_job(Compare, "job_4"),
            with_job(Gate, "job_5"),
            ev(Reallocate),
            ev(Hypothesize),
        ];
        let s = fold(&log).unwrap();
        assert_eq!(s.phase, Hypothesize);
        assert_eq!(s.seq, 9);
        assert_eq!(s.cycles, 1);
        assert!(!s.is_terminal());
        assert_eq!(s.children.len(), 5);
        assert_eq!(s.next_phase(), Some(Experiment));
    }

    /// AT-59 — a driver killed mid-campaign and restarted folds the same state,
    /// and therefore keys the same child job. Nothing is remembered between the
    /// two; the log is the memory.
    #[test]
    fn replay_is_identical_and_keys_the_same_child() {
        let id = Uuid::new_v4();
        let log = vec![ev(Define), with_job(Baseline, "job_1"), ev(Diagnose), ev(Hypothesize)];
        let before = fold(&log).unwrap();
        let after = fold(&log.clone()).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            before.child_key(id, Experiment),
            after.child_key(id, Experiment),
            "the same log must key the same child job or a restart runs the phase twice"
        );
        // A different campaign, phase or position is a different key.
        assert_ne!(before.child_key(id, Experiment), before.child_key(Uuid::new_v4(), Experiment));
        assert_ne!(before.child_key(id, Experiment), before.child_key(id, Compare));
        let advanced = fold(&[log.clone(), vec![ev(Experiment)]].concat()).unwrap();
        assert_ne!(before.child_key(id, Experiment), advanced.child_key(id, Experiment));
    }

    #[test]
    fn an_illegal_transition_is_refused() {
        let err = fold(&[ev(Define), ev(Gate)]).unwrap_err();
        assert!(format!("{err}").contains("cannot follow"), "{err}");
    }

    #[test]
    fn a_campaign_can_end_from_anywhere_but_not_continue_after() {
        for end in CampaignPhase::TERMINAL {
            let s = fold(&[ev(Define), ev(Baseline), ev(end)]).unwrap();
            assert_eq!(s.terminal, Some(end));
            assert_eq!(s.next_phase(), None);

            let err = fold(&[ev(Define), ev(Baseline), ev(end), ev(Diagnose)]).unwrap_err();
            assert!(format!("{err}").contains("appended after"), "{err}");
        }
    }

    #[test]
    fn only_the_phases_that_dispatch_work_have_child_jobs() {
        for p in CampaignPhase::ALL {
            assert_eq!(
                p.has_child_job(),
                matches!(p, Baseline | Diagnose | Experiment | Compare | Gate),
                "{p:?}"
            );
        }
    }
}

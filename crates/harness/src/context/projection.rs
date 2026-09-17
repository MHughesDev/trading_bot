//! Sub-run context is a projection, not a second pipeline (AGENT-005 §29,
//! §30.20; AGENT-010 §9; Set S task S-9.20).
//!
//! # The gap
//!
//! AGENT-010 §9 gives a critic "the evidence bundle only" and a worker "a
//! byte-identical brief", and says nothing about how either is built. Built by
//! hand they would be a second context pipeline with none of this one's
//! guarantees: no manifest, no declared degradation, no selection report, and no
//! way to check afterwards what the critic actually saw.
//!
//! So a sub-run's context is a **projection of the parent store** — a filter plus
//! a budget, through the same [`select`](super::select), the same
//! [`degrade`](super::degrade) and the same [`manifest`](super::manifest).
//!
//! # "Fresh context" becomes a mechanism rather than an instruction
//!
//! A critic is supposed to judge the evidence without being led by the reasoning
//! that produced it. Told as an instruction that is a request the model may or
//! may not honour, and nobody can check afterwards which it did.
//!
//! [`Projection::Critic`] excludes the parent's reasoning and corrections **by
//! construction**: the records are not filtered out of the prompt, they are not
//! in the store the critic selects from. A critic that saw too much is then
//! impossible rather than arguable — and the projection used is recorded on the
//! sub-run, so the claim is checkable after the fact either way.
//!
//! # Why the worker projection is pinned
//!
//! Fan-out is only cache-friendly if every worker's brief is the same bytes. A
//! projection that selected per worker would give each one a slightly different
//! prefix and pay a cold prefix per worker — which on a twelve-way fan-out is
//! twelve.

use std::collections::BTreeSet;

use super::store::{Record, RecordId, RecordKind, RunStore};

/// What a sub-run is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    /// The evidence bundle: findings, the results they cite, and memory.
    ///
    /// Excludes the parent's reasoning — decisions, plan changes, harness notes
    /// — and its corrections. A correction is a record of the parent being told
    /// it was wrong about *how to call a tool*; handing it to a critic tells the
    /// critic what the parent already concluded about its own process, which is
    /// the influence the fresh context exists to prevent.
    Critic,
    /// A fixed set, identical for every worker in a fan-out.
    Worker { pinned: BTreeSet<RecordId> },
    /// Everything the parent has. For a sub-run that is a continuation rather
    /// than a fresh look — named so it is a *choice* rather than the absence of
    /// one.
    Full,
}

impl Projection {
    /// A worker projection over the ids every worker must share.
    #[must_use]
    pub fn worker(ids: impl IntoIterator<Item = RecordId>) -> Self {
        Projection::Worker {
            pinned: ids.into_iter().collect(),
        }
    }

    /// The name recorded on the sub-run, so what it saw is checkable later.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Projection::Critic => "critic",
            Projection::Worker { .. } => "worker",
            Projection::Full => "full",
        }
    }

    /// Whether this record reaches the sub-run.
    #[must_use]
    pub fn admits(&self, r: &Record) -> bool {
        match self {
            Projection::Critic => matches!(
                r.kind,
                RecordKind::Finding | RecordKind::ToolResult | RecordKind::Memory
            ),
            Projection::Worker { pinned } => pinned.contains(&r.id),
            Projection::Full => true,
        }
    }
}

/// What a sub-run receives, and the account of how it was cut.
#[derive(Debug, Clone)]
pub struct Projected {
    pub store: RunStore,
    /// Recorded on the sub-run (§29).
    pub projection: &'static str,
    /// Records the parent had and the sub-run does not.
    pub withheld: Vec<RecordId>,
}

/// Project the parent's store for a sub-run.
#[must_use]
pub fn project(parent: &RunStore, projection: &Projection) -> Projected {
    let store = parent.subset(|r| projection.admits(r));
    let kept: BTreeSet<RecordId> = store.all().map(|r| r.id).collect();
    Projected {
        withheld: parent
            .all()
            .map(|r| r.id)
            .filter(|id| !kept.contains(id))
            .collect(),
        store,
        projection: projection.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Body;
    use crate::context::policy::ContextPolicy;
    use crate::context::render;
    use crate::context::select::{select, Query};
    use crate::context::store::{Draft, Subject};
    use crate::state::GoalKind;

    fn parent() -> RunStore {
        let mut s = RunStore::new();
        s.insert(
            Draft::new(1, RecordKind::ToolResult, "run_backtest", Body::text("sharpe 1.4"))
                .with_subject(Subject::instrument("BTC-USD"))
                .with_id("experiment_id", "exp_7"),
        );
        s.insert(
            Draft::new(2, RecordKind::Finding, "record_finding", Body::text("momentum holds"))
                .with_subject(Subject::instrument("BTC-USD")),
        );
        s.insert(Draft::new(
            3,
            RecordKind::Decision,
            "harness",
            Body::text("chose the 1h timeframe because the 1d sample was thin"),
        ));
        s.insert(
            Draft::new(4, RecordKind::Correction, "validate_strategy", Body::text("bad name"))
                .with_id("code", "when_names_string"),
        );
        s.insert(Draft::new(
            5,
            RecordKind::PlanChange,
            "harness",
            Body::text("dropped the funding leg"),
        ));
        s.insert(Draft::new(
            6,
            RecordKind::Memory,
            "memory",
            Body::text("a prior finding about BTC-USD"),
        ));
        s
    }

    /// A critic provably cannot see the parent's reasoning or corrections.
    ///
    /// By construction, not by instruction: the records are not in the store the
    /// critic selects from, so "fresh context" is a mechanism rather than a
    /// request the model may or may not honour.
    #[test]
    fn a_critic_cannot_see_the_parents_reasoning() {
        let p = parent();
        let projected = project(&p, &Projection::Critic);
        for r in projected.store.all() {
            assert!(
                !matches!(
                    r.kind,
                    RecordKind::Decision | RecordKind::PlanChange | RecordKind::Correction
                ),
                "{:?} reached the critic",
                r.kind
            );
        }
        // And what it does get: the evidence bundle.
        let kinds: BTreeSet<RecordKind> = projected.store.all().map(|r| r.kind).collect();
        assert!(kinds.contains(&RecordKind::Finding));
        assert!(kinds.contains(&RecordKind::ToolResult));
        assert!(kinds.contains(&RecordKind::Memory));
        assert_eq!(projected.projection, "critic");
        assert_eq!(
            projected.withheld.len(),
            3,
            "and what was withheld is recorded, so the claim is checkable"
        );
    }

    /// A critic still gets the *results its findings cite*, not findings alone.
    ///
    /// A critic handed conclusions without the evidence can only agree or
    /// disagree with the parent's judgement, which is the one thing it is not
    /// there to do.
    #[test]
    fn a_critic_gets_the_evidence_and_not_only_the_conclusions() {
        let projected = project(&parent(), &Projection::Critic);
        assert!(
            projected
                .store
                .all()
                .any(|r| r.ids.get("experiment_id").map(String::as_str) == Some("exp_7")),
            "the result the finding rests on must be there"
        );
    }

    /// Two workers get byte-identical briefs (§29).
    #[test]
    fn every_worker_gets_the_same_bytes() {
        let p = parent();
        let ids: Vec<RecordId> = p
            .all()
            .filter(|r| r.kind == RecordKind::Finding || r.kind == RecordKind::ToolResult)
            .map(|r| r.id)
            .collect();
        let projection = Projection::worker(ids);

        let policy = ContextPolicy::v2();
        let brief = |worker_subject: &str| {
            let projected = project(&p, &projection);
            // Deliberately different queries: the workers are doing different
            // things. The brief must still be identical, which is what makes a
            // fan-out cache-friendly.
            let q = Query {
                step: 1,
                goal_kind: GoalKind::Research,
                ..Query::default()
            }
            .with_subject(Subject::instrument(worker_subject));
            let sel = select(&projected.store, &q, 100_000, &policy);
            render::render(&projected.store, &sel, &policy, "").text
        };

        assert_eq!(
            brief("BTC-USD"),
            brief("ETH-USD"),
            "a projection that selected per worker would pay a cold prefix per worker"
        );
    }

    /// The projection is recorded, so a critic that saw too much is visible.
    #[test]
    fn the_projection_used_is_recorded() {
        assert_eq!(project(&parent(), &Projection::Critic).projection, "critic");
        assert_eq!(
            project(&parent(), &Projection::worker([])).projection,
            "worker"
        );
        assert_eq!(project(&parent(), &Projection::Full).projection, "full");
    }

    /// Ids survive the projection.
    ///
    /// A critic citing `r007` and the parent that wrote `r007` must mean the same
    /// observation, or the citation cannot be checked against the run that
    /// produced it.
    #[test]
    fn ids_are_preserved_rather_than_renumbered() {
        let p = parent();
        let projected = project(&p, &Projection::Critic);
        for r in projected.store.all() {
            let original = p.get(r.id).expect("the parent has it under the same id");
            assert_eq!(original.body, r.body);
            assert_eq!(original.step, r.step);
        }
    }

    /// The whole parent is a named choice, not the default.
    #[test]
    fn full_access_is_something_a_caller_has_to_ask_for() {
        let p = parent();
        let full = project(&p, &Projection::Full);
        assert_eq!(full.store.len(), p.len());
        assert!(full.withheld.is_empty());
    }
}

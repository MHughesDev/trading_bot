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

/// The kinds a campaign's HYPOTHESIZE worker may see (AGENT-013 §8.5).
///
/// Findings, the results they cite — the diagnostic bundle and the parent
/// candidate and its cousins arrive as `ToolResult`s (FEAT-003 §8.2 step 2) —
/// and memory. Not the parent's reasoning: not its decisions, not its plan
/// changes, not the corrections it collected while learning to call tools.
///
/// The exclusion is the same one [`Projection::Critic`] makes and it is made the
/// same way, by construction rather than by instruction. A worker that proposed
/// the move the parent had already talked itself into would be a very expensive
/// way of asking the parent again.
pub const CAMPAIGN_WORKER_KINDS: &[RecordKind] = &[
    RecordKind::Finding,
    RecordKind::ToolResult,
    RecordKind::Memory,
];

/// Pin a campaign worker's projection from the parent's store.
///
/// **Derived, never listed.** The alternative — a caller assembling the id set
/// by hand — is how a `Decision` ends up pinned: not by anybody deciding the
/// worker should see the parent's reasoning, but by a filter somewhere else
/// changing and this list not. The ids come out of the store through
/// [`CAMPAIGN_WORKER_KINDS`], so there is one rule and it is this one.
#[must_use]
pub fn campaign_worker(parent: &RunStore) -> Projection {
    Projection::worker(
        parent
            .all()
            .filter(|r| CAMPAIGN_WORKER_KINDS.contains(&r.kind))
            .map(|r| r.id),
    )
}

/// A sub-run's brief: the bytes, and the account of how they were chosen.
///
/// `text` is what the sub-run is given. It comes out of the same
/// [`select`](super::select) → [`degrade`](super::degrade) →
/// [`render`](super::render) pipeline the parent's own prompts do, over the
/// projected store — which is the whole point of projecting rather than
/// assembling: a hand-built brief would have no manifest, no declared
/// degradation and no selection report, and nothing afterwards could say what
/// the sub-run actually saw.
#[derive(Debug, Clone)]
pub struct Brief {
    pub text: String,
    /// The index over the projected store, including what was elided.
    pub manifest: String,
    /// The records rendered, in render order.
    pub shown: Vec<RecordId>,
    /// Recorded on the sub-run (§29).
    pub projection: &'static str,
    /// What the parent had and the sub-run does not.
    pub withheld: Vec<RecordId>,
    /// The selection report, so "why was this in the brief" has an answer.
    pub report: super::select::SelectionReport,
}

/// Assemble a sub-run's brief through the parent's own pipeline.
#[must_use]
pub fn brief(
    projected: &Projected,
    query: &super::select::Query,
    budget: u32,
    policy: &super::policy::ContextPolicy,
    manifest_tokens: u32,
) -> Brief {
    let selection = super::select::select(&projected.store, query, budget, policy);
    let shown = selection.ids();
    let index = super::manifest::render(
        &projected.store,
        &shown.iter().copied().collect(),
        manifest_tokens,
    );
    let rendered = super::render::render(&projected.store, &selection, policy, &index.text);
    Brief {
        text: rendered.text,
        manifest: index.text,
        shown,
        projection: projected.projection,
        withheld: projected.withheld.clone(),
        report: selection.report,
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

    /// S-10.11's acceptance, as a mechanism: the campaign's HYPOTHESIZE worker
    /// provably cannot see the parent's reasoning.
    ///
    /// This is S-9.20's critic test pointed at the other projection, and it
    /// matters more here — a critic that was led would produce a soft verdict,
    /// while a worker that was led produces the parent's own move wearing a
    /// second opinion's name, and the campaign would record it as an
    /// independent proposal.
    #[test]
    fn a_campaign_worker_cannot_see_the_parents_reasoning() {
        let p = parent();
        let projection = campaign_worker(&p);
        assert_eq!(projection.as_str(), "worker");
        let projected = project(&p, &projection);
        for r in projected.store.all() {
            assert!(
                !matches!(
                    r.kind,
                    RecordKind::Decision | RecordKind::PlanChange | RecordKind::Correction
                ),
                "{:?} reached the campaign worker",
                r.kind
            );
        }
        let kinds: BTreeSet<RecordKind> = projected.store.all().map(|r| r.kind).collect();
        assert!(kinds.contains(&RecordKind::Finding), "findings are the point");
        assert!(
            kinds.contains(&RecordKind::ToolResult),
            "the diagnostic bundle and the parent candidate arrive as tool results"
        );
        assert!(kinds.contains(&RecordKind::Memory));
        assert_eq!(
            projected.withheld.len(),
            3,
            "and what was withheld is recorded, so the claim is checkable"
        );
    }

    /// The pinned set is derived from the store, not handed in.
    ///
    /// A worker projection admits exactly the ids it was given, so a caller that
    /// assembled the set by hand could pin a `Decision` and nothing would stop
    /// it. `campaign_worker` is the single place that rule lives, and adding a
    /// reasoning record to the parent must never add one to the brief.
    #[test]
    fn adding_reasoning_to_the_parent_never_widens_the_worker_brief() {
        let mut p = parent();
        let before = project(&p, &campaign_worker(&p)).store.len();
        p.insert(Draft::new(
            7,
            RecordKind::Decision,
            "harness",
            Body::text("decided to try a regime gate next"),
        ));
        p.insert(Draft::new(
            8,
            RecordKind::Correction,
            "propose_move",
            Body::text("that node does not exist"),
        ));
        let after = project(&p, &campaign_worker(&p));
        assert_eq!(after.store.len(), before);
        assert_eq!(after.withheld.len(), 5);
    }

    /// The brief comes out of the parent's own pipeline, so it has a manifest
    /// and a selection report rather than being a string somebody concatenated.
    #[test]
    fn a_brief_carries_its_manifest_and_its_reasons() {
        let p = parent();
        let projected = project(&p, &campaign_worker(&p));
        let policy = ContextPolicy::v1();
        let q = Query {
            step: 1,
            goal_kind: GoalKind::Research,
            subject: Subject::instrument("BTC-USD"),
            ..Query::default()
        };
        let b = brief(&projected, &q, 2_000, &policy, 200);
        assert_eq!(b.projection, "worker");
        assert!(!b.shown.is_empty(), "the worker was handed nothing at all");
        assert!(!b.manifest.is_empty(), "a brief with no index is a wall of text");
        assert_eq!(b.report.candidates, projected.store.len());
        for id in &b.shown {
            let r = projected.store.get(*id).expect("rendered from the projection");
            assert!(
                CAMPAIGN_WORKER_KINDS.contains(&r.kind),
                "{:?} was rendered into the brief",
                r.kind
            );
        }
        assert_eq!(b.withheld.len(), 3);
    }

    /// Two workers off one projection get the same bytes.
    ///
    /// Fan-out is only cache-friendly if the brief is identical; a brief that
    /// varied per worker would pay a cold prefix per worker, which on a
    /// twelve-way fan-out is twelve.
    #[test]
    fn two_workers_off_one_projection_get_identical_bytes() {
        let p = parent();
        let projected = project(&p, &campaign_worker(&p));
        let policy = ContextPolicy::v1();
        let q = Query {
            step: 1,
            goal_kind: GoalKind::Research,
            subject: Subject::instrument("BTC-USD"),
            ..Query::default()
        };
        let a = brief(&projected, &q, 2_000, &policy, 200);
        let b = brief(&projected, &q, 2_000, &policy, 200);
        assert_eq!(a.text, b.text);
        assert_eq!(a.manifest, b.manifest);
    }
}

//! What would have helped — counterfactual ablation (AGENT-005 §28.1; Set S task
//! S-9.19).
//!
//! # Why §19 is not enough
//!
//! [`utility`](super::utility) measures what the model *used*. That is not what
//! it *needed*. A record can be used because it was there and a better one was
//! not; a record can go uncited because a worse one filled the slot first.
//! Citation counts reward whatever selection already chose, which makes them a
//! poor signal for fitting selection.
//!
//! So: over recorded tapes, re-run the assembly with one selected record removed
//! and compare. A record whose removal changes nothing was not load-bearing,
//! whatever attribution said about it.
//!
//! This is cheap **exactly here and nowhere else**: the loop is sans-IO, the tape
//! supplies the model's reply, and selection makes no provider call. It is
//! `ablation_sweep` pointed at the platform's own prompt.
//!
//! # What this can and cannot conclude — stated, because it matters
//!
//! With a tape the model's reply is fixed, so the true counterfactual — what the
//! model *would have done* with a different prompt — is not available without
//! re-running the model. What is available is whether the step could still have
//! been taken: whether the evidence the actual call rested on survives the
//! removal.
//!
//! That is a **necessary** condition for load-bearing, not a sufficient one. A
//! record whose removal leaves every cited id still present may still have
//! changed the model's reasoning, and this will call it incidental. The bound is
//! one-sided and it is the safe side: [`Verdict::LoadBearing`] is never claimed
//! on evidence that does not support it. `ADR-S9-04` records the limit rather
//! than leaving a reader to infer that a stronger claim was intended.

use std::collections::BTreeSet;

use super::policy::ContextPolicy;
use super::select::{select, Query};
use super::store::{RecordId, RunStore};

/// What removing one record did to the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The call the model actually made rested on evidence this record carried,
    /// and without it that evidence is not in the prompt.
    LoadBearing,
    /// Removing it changed which other records were rendered, but everything the
    /// call rested on is still there.
    ChangedTheSet,
    /// Nothing the call rested on moved, and the rest of the set is unchanged.
    Incidental,
}

/// One record's ablation result.
#[derive(Debug, Clone)]
pub struct Ablated {
    pub id: RecordId,
    pub verdict: Verdict,
    /// Records that entered the prompt because this one left it.
    pub replaced_by: Vec<RecordId>,
    /// Ids the actual call rested on that the ablated prompt no longer carries.
    pub evidence_lost: Vec<String>,
}

/// Run the sweep over one recorded step.
///
/// `cited` is what the model's actual next call rested on — the id strings it
/// named. It comes from the tape, never from a fresh model call.
#[must_use]
pub fn sweep(
    store: &RunStore,
    query: &Query,
    budget: u32,
    policy: &ContextPolicy,
    cited: &BTreeSet<String>,
) -> Vec<Ablated> {
    let base = select(store, query, budget, policy);
    let base_ids: BTreeSet<RecordId> = base.ids().into_iter().collect();
    // What the step actually had. Evidence the base prompt never carried cannot
    // be *lost* by a removal — and without this line every record reads as
    // load-bearing on any step whose cited id the budget had already squeezed
    // out, which is the majority of the interesting steps. Found by the tight-
    // budget test, where two unrelated records were both reported as carrying an
    // id neither of them had.
    let base_evidence: BTreeSet<&String> = base_ids
        .iter()
        .filter_map(|i| store.get(*i))
        .flat_map(|r| r.ids.values())
        .collect();
    let cited: BTreeSet<String> = cited
        .iter()
        .filter(|c| base_evidence.contains(*c))
        .cloned()
        .collect();

    let mut out = Vec::with_capacity(base_ids.len());
    for id in &base_ids {
        let mut q = query.clone();
        q.excluded.insert(*id);
        let without = select(store, &q, budget, policy);
        let ids: BTreeSet<RecordId> = without.ids().into_iter().collect();

        let replaced_by: Vec<RecordId> = ids.difference(&base_ids).copied().collect();
        // Which of the call's ids survive in the ablated prompt.
        let still_present: BTreeSet<&String> = ids
            .iter()
            .filter_map(|i| store.get(*i))
            .flat_map(|r| r.ids.values())
            .collect();
        let evidence_lost: Vec<String> = cited
            .iter()
            .filter(|c| !still_present.contains(*c))
            .cloned()
            .collect();

        let verdict = if evidence_lost.is_empty() {
            if replaced_by.is_empty() {
                Verdict::Incidental
            } else {
                Verdict::ChangedTheSet
            }
        } else {
            Verdict::LoadBearing
        };
        out.push(Ablated {
            id: *id,
            verdict,
            replaced_by,
            evidence_lost,
        });
    }
    out
}

/// The load-bearing records, most consequential first.
///
/// Ties break on the record id, so the ranking is reproducible from the corpus —
/// which is the acceptance, and which a sort by an `f32` score alone would not
/// give.
#[must_use]
pub fn ranking(ablated: &[Ablated]) -> Vec<RecordId> {
    let mut rows: Vec<&Ablated> = ablated.iter().collect();
    rows.sort_by_key(|a| {
        let rank = match a.verdict {
            Verdict::LoadBearing => 0,
            Verdict::ChangedTheSet => 1,
            Verdict::Incidental => 2,
        };
        (rank, std::cmp::Reverse(a.evidence_lost.len()), a.id)
    });
    rows.into_iter().map(|a| a.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Body;
    use crate::context::store::{Draft, RecordKind, Subject};
    use crate::state::GoalKind;

    fn fixture() -> (RunStore, RecordId, RecordId) {
        let mut s = RunStore::new();
        // The record the call rests on: it carries the experiment id.
        let bearing = s.insert(
            Draft::new(
                2,
                RecordKind::ToolResult,
                "run_backtest",
                Body::text("x".repeat(200)),
            )
            .with_subject(Subject::instrument("BTC-USD"))
            .with_id("experiment_id", "exp_7"),
        );
        // Plenty of plausible neighbours the call never used.
        let mut incidental = bearing;
        for i in 0..6u32 {
            incidental = s.insert(
                Draft::new(
                    3 + i,
                    RecordKind::ToolResult,
                    "get_bars",
                    Body::text("y".repeat(200)),
                )
                .with_subject(Subject::instrument(format!("SYM-{i}")))
                .with_id("window", format!("w{i}")),
            );
        }
        (s, bearing, incidental)
    }

    fn query() -> Query {
        Query {
            step: 10,
            goal_kind: GoalKind::Research,
            ..Query::default()
        }
    }

    /// The sweep runs with no provider call. The property that makes it cheap.
    #[test]
    fn the_sweep_makes_no_model_call() {
        let (s, bearing, _) = fixture();
        let cited = BTreeSet::from(["exp_7".to_string()]);
        let rows = sweep(&s, &query(), 100_000, &ContextPolicy::v2(), &cited);
        assert!(!rows.is_empty());
        assert!(rows.iter().any(|r| r.id == bearing));
    }

    /// A record the call rested on is load-bearing; its neighbours are not.
    #[test]
    fn removal_separates_load_bearing_from_incidental() {
        let (s, bearing, incidental) = fixture();
        let cited = BTreeSet::from(["exp_7".to_string()]);
        let rows = sweep(&s, &query(), 100_000, &ContextPolicy::v2(), &cited);

        let b = rows.iter().find(|r| r.id == bearing).expect("swept");
        assert_eq!(b.verdict, Verdict::LoadBearing);
        assert_eq!(b.evidence_lost, vec!["exp_7".to_string()]);

        let i = rows.iter().find(|r| r.id == incidental).expect("swept");
        assert_ne!(
            i.verdict,
            Verdict::LoadBearing,
            "a record whose removal leaves the call's evidence intact was not load-bearing, \
             whatever attribution said"
        );
    }

    /// The load-bearing ranking differs from the citation ranking (§28.1).
    ///
    /// This is the acceptance, and it is the whole reason for the module: if the
    /// two rankings agreed, attribution would already answer the question and
    /// there would be nothing to ablate.
    #[test]
    fn the_load_bearing_ranking_differs_from_the_citation_ranking() {
        let (mut s, bearing, _) = fixture();
        // A record the model cited twice and that carries nothing the call
        // needed — high on citations, incidental under ablation. The shape is
        // ordinary: a heading the model keeps quoting back.
        let chatty = s.insert(
            Draft::new(4, RecordKind::Note, "harness", Body::text("z".repeat(200)))
                .with_subject(Subject::instrument("BTC-USD")),
        );
        s.note_citation(chatty);
        s.note_citation(chatty);

        let cited = BTreeSet::from(["exp_7".to_string()]);
        let rows = sweep(&s, &query(), 100_000, &ContextPolicy::v2(), &cited);
        let by_ablation = ranking(&rows);

        let mut by_citation: Vec<RecordId> = rows.iter().map(|r| r.id).collect();
        by_citation.sort_by_key(|id| std::cmp::Reverse(s.get(*id).map_or(0, |r| r.citations)));

        assert_eq!(by_ablation.first(), Some(&bearing), "{by_ablation:?}");
        assert_eq!(by_citation.first(), Some(&chatty), "{by_citation:?}");
        assert_ne!(
            by_ablation, by_citation,
            "if these agreed there would be nothing to ablate"
        );
    }

    /// Removing a record can pull another in, and that is reported separately.
    #[test]
    fn a_replacement_is_reported_rather_than_read_as_no_change() {
        let (s, _, _) = fixture();
        let cited = BTreeSet::from(["exp_7".to_string()]);
        // A budget tight enough that the set is a choice rather than everything.
        let rows = sweep(&s, &query(), 120, &ContextPolicy::v2(), &cited);
        assert!(
            rows.iter()
                .any(|r| r.verdict == Verdict::ChangedTheSet && !r.replaced_by.is_empty()),
            "under a tight budget some removal must let something else in: {rows:?}"
        );
    }
}

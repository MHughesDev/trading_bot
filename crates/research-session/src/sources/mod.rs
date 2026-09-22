//! The subscriptions: the sources that put work on the agenda without anybody
//! asking (AGENT-014 §4.3–4.7).
//!
//! Sources 4.1 (charter seeds) and 4.2 (the user) are pushes — something
//! happens once and an item exists. Source 4.8 is a push too: a finished run's
//! next moves are offered by [`crate::harvest`] at the moment the run ends.
//! What is here is the other shape. §4's own sentence is *"Sources 4.3–4.7 are
//! subscriptions: the driver evaluates them on every iteration"* — they are
//! **pulls**, re-evaluated against memory, the ledger and the world, and they
//! are what makes a session react to the market rather than to its own plan.
//!
//! # Every source ends at [`crate::agenda::offer`]
//!
//! There is no second dedupe in this module and there is no path that inserts
//! an item without passing layer one. Five subscriptions firing on the same
//! iteration over an overlapping scope is the ordinary case, not the unusual
//! one, and a subscription that inserted directly would give the same question
//! five cards.
//!
//! # An absent measurement is never a zero, and never a "no"
//!
//! A subscription asks a question about the world — *has the data window
//! extended?*, *has the regime flipped?* — and the world sometimes has no
//! answer on file. [`Fired::NotEvaluable`] is that case, and it is a **third**
//! value rather than a `false`, because the two lead to different behaviour:
//! a measured `no` is silence, and an unmeasurable condition is a line on the
//! log saying the session is carrying a rule it cannot evaluate. Folding the
//! second into the first is how a subscription comes to look like it is working
//! when nothing has ever asked it anything.

pub mod ledger;
pub mod memory;
pub mod observe;

use crate::agenda::{offer, AgendaItem, Insert};
use crate::charter::Charter;
use crate::harvest::{SkipReason, Skipped};

/// Whether a subscription's condition holds right now.
///
/// Three values, not two. See the module header: absence is not denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fired {
    /// The condition is measured and true.
    Yes,
    /// The condition is measured and false.
    No,
    /// Nothing on this platform can answer the question. Carries what was
    /// missing, because a subscription that cannot fire is worth a line in the
    /// digest and worth a spec amendment, and neither happens if it looks
    /// identical to one that fired and said no.
    NotEvaluable { missing: String },
}

impl Fired {
    /// Whether this should put an item on the agenda.
    ///
    /// [`Fired::NotEvaluable`] answers **no**, and does so through a method
    /// whose name says it is about inserting rather than about truth — the two
    /// are the same here and are not the same in the digest.
    #[must_use]
    pub fn inserts(&self) -> bool {
        matches!(self, Fired::Yes)
    }

    /// The word the log records.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Fired::Yes => "yes",
            Fired::No => "no",
            Fired::NotEvaluable { .. } => "not_evaluable",
        }
    }
}

/// What one iteration's subscriptions did.
///
/// The same shape [`crate::harvest::Harvest`] returns, for the same reason: the
/// driver writes one row per [`Insert`] and one note per [`Skipped`], and a
/// candidate that was dropped is a line rather than a silence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ingest {
    pub inserted: Vec<Insert>,
    pub skipped: Vec<Skipped>,
}

impl Ingest {
    /// The items that are genuinely new.
    pub fn added(&self) -> impl Iterator<Item = &AgendaItem> {
        self.inserted.iter().filter_map(|i| match i {
            Insert::Added(item) => Some(&**item),
            Insert::Merged { .. } => None,
        })
    }

    /// How many offers layer one merged rather than inserted.
    #[must_use]
    pub fn merged_count(&self) -> usize {
        self.inserted
            .iter()
            .filter(|i| matches!(i, Insert::Merged { .. }))
            .count()
    }

    fn absorb(&mut self, other: Ingest) {
        self.inserted.extend(other.inserted);
        self.skipped.extend(other.skipped);
    }
}

/// Offer a batch of candidates to the agenda through layer one.
///
/// The agenda is grown as it goes — an item added by the first candidate is
/// visible to the second — because two subscriptions proposing the same
/// sentence on one iteration is the case layer one exists for, and it would
/// miss it entirely if every candidate were compared against the agenda as it
/// stood before the batch.
pub(crate) fn offer_all(
    agenda: &mut Vec<AgendaItem>,
    candidates: impl IntoIterator<Item = AgendaItem>,
) -> Vec<Insert> {
    let mut out = Vec::new();
    for item in candidates {
        let result = offer(agenda.as_mut_slice(), item);
        if let Insert::Added(added) = &result {
            agenda.push((**added).clone());
        }
        out.push(result);
    }
    out
}

/// Whether the charter lets this subject and workflow onto the agenda at all,
/// and the [`Skipped`] line if not.
///
/// Applied by every subscription before it builds an item, because a
/// subscription that ingests out-of-scope work and relies on the scorer to give
/// it a zero has put a card on the human's board that nothing will ever claim.
pub(crate) fn admitted(charter: &Charter, subject: &str, label: &str) -> Result<(), Skipped> {
    if charter.excludes(subject) {
        return Err(Skipped {
            what: label.to_string(),
            why: SkipReason::Excluded,
        });
    }
    if !charter.scope.admits(subject) {
        return Err(Skipped {
            what: label.to_string(),
            why: SkipReason::OutOfScope,
        });
    }
    Ok(())
}

/// Everything the driver measured this iteration, in one place.
///
/// Deliberately a plain struct of already-read facts rather than a trait with
/// async readers: every rule in this module is a decision, decisions test
/// without a database, and the crate's other four pure modules are built the
/// same way. The driver's job is to fill this; what to do with it is decided
/// here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct World {
    /// Open questions in the session's scope (§4.3).
    pub open_questions: Vec<memory::QuestionRow>,
    /// Negative results whose retry condition the driver wants evaluated (§4.4).
    pub negatives: Vec<ledger::NegativeRow>,
    /// Contradiction pairs with no resolving finding (§4.5).
    pub contradictions: Vec<memory::ContradictionPair>,
    /// `established` findings past `valid_until` (§4.6).
    pub stale_findings: Vec<memory::FindingRow>,
    /// What the world now measures, for the conditions in §4.4 that have an
    /// operand to compare against.
    pub measured: ledger::Measured,
    /// Observations since the last iteration (§4.7).
    pub observations: Vec<observe::Observation>,
    /// The fingerprints this session has already raised an item for, so a
    /// standing observation does not become one item per profiler run.
    pub already_raised: observe::Raised,
}

/// Run every subscription for one iteration (AGENT-014 §4.3–4.7).
///
/// The order is fixed and is not alphabetical: it is §4's own table order, so
/// that when two sources want the same sentence the **earlier, heavier** source
/// is the one that creates the item and the later one merges onto it. A
/// blocking open question and a refinement follow-up asking the same thing
/// should be one card whose first source is the blocking question, and the
/// order of a `BTreeMap` iteration is not a reason for it to be the other way
/// round.
#[must_use]
pub fn ingest(charter: &Charter, world: &World, agenda: &mut Vec<AgendaItem>) -> Ingest {
    let mut out = Ingest::default();
    out.absorb(memory::open_questions(charter, &world.open_questions, agenda));
    out.absorb(ledger::retry_conditions(
        charter,
        &world.negatives,
        &world.measured,
        agenda,
    ));
    out.absorb(memory::contradictions(
        charter,
        &world.contradictions,
        agenda,
    ));
    out.absorb(memory::stale_findings(
        charter,
        &world.stale_findings,
        agenda,
    ));
    out.absorb(observe::observations(
        charter,
        &world.observations,
        &world.already_raised,
        agenda,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::{ItemStatus, Source};
    use crate::charter::{Objective, Scope};
    use uuid::Uuid;

    /// Session ids are fixed in every test in this crate. A previous track drew
    /// a fresh `Uuid::new_v4()` inside a statistical assertion and shipped a
    /// test that failed about one run in seven.
    pub(super) fn session() -> Uuid {
        Uuid::from_u128(0x5e55_1041)
    }

    pub(super) fn charter_over(subjects: &[&str]) -> Charter {
        Charter {
            scope: Scope {
                instruments: subjects.iter().map(|s| (*s).to_string()).collect(),
                ..Scope::default()
            },
            objective: Objective::BuildKnowledge {
                about: subjects.iter().map(|s| (*s).to_string()).collect(),
            },
            seeds: vec![],
            permitted_workflows: vec![],
            exclusions: vec![],
            standing_constraints: vec![],
            exploration_share: crate::charter::ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        }
    }

    #[test]
    fn two_subscriptions_wanting_the_same_sentence_make_one_item_with_two_sources() {
        let charter = charter_over(&["btc"]);
        let world = World {
            open_questions: vec![memory::QuestionRow::new(
                "q_1",
                "does funding predict returns",
                "btc",
            )],
            // A contradiction phrased identically. In the wild this is what two
            // subscriptions over an overlapping scope look like.
            contradictions: vec![memory::ContradictionPair::new(
                "f_a",
                "f_b",
                "btc",
                "does funding predict returns",
            )],
            ..World::default()
        };
        let mut agenda = Vec::new();
        let out = ingest(&charter, &world, &mut agenda);

        assert_eq!(agenda.len(), 1, "one item, not two: {agenda:?}");
        assert_eq!(out.merged_count(), 1);
        assert_eq!(
            agenda[0].sources,
            vec![Source::OpenQuestion, Source::Contradiction],
            "the open question created it and the contradiction merged onto it"
        );
        assert!(
            agenda[0].source_refs.iter().any(|r| r == "q_1"),
            "the question's row id"
        );
    }

    #[test]
    fn a_terminal_item_does_not_swallow_a_fresh_offer() {
        // §4's rule is "any item in a *non-terminal* status". A question that
        // was answered and is being asked again is replication, and replication
        // is something the platform is for.
        let charter = charter_over(&["btc"]);
        let world = World {
            open_questions: vec![memory::QuestionRow::new("q_1", "does x hold", "btc")],
            ..World::default()
        };
        let mut agenda = vec![AgendaItem {
            status: ItemStatus::Done {
                verdict: "reject".into(),
            },
            ..AgendaItem::new(session(), "does x hold", "btc", Source::CharterSeed)
        }];
        let out = ingest(&charter, &world, &mut agenda);
        assert_eq!(out.merged_count(), 0);
        assert_eq!(agenda.len(), 2);
    }

    #[test]
    fn nothing_out_of_scope_reaches_the_agenda_and_each_drop_is_a_line() {
        let charter = charter_over(&["btc"]);
        let world = World {
            open_questions: vec![memory::QuestionRow::new("q_1", "does x hold", "eth")],
            stale_findings: vec![memory::FindingRow::new("f_1", "eth", "x holds")],
            ..World::default()
        };
        let mut agenda = Vec::new();
        let out = ingest(&charter, &world, &mut agenda);
        assert!(agenda.is_empty());
        assert_eq!(out.skipped.len(), 2, "{:?}", out.skipped);
        assert!(out
            .skipped
            .iter()
            .all(|s| s.why == SkipReason::OutOfScope));
    }

    #[test]
    fn an_exclusion_is_refused_under_its_own_word_and_not_as_out_of_scope() {
        // The two read the same on an agenda and completely differently to a
        // human: one is "you did not ask about this" and the other is "you told
        // me not to".
        let mut charter = charter_over(&["btc"]);
        charter.exclusions.push("btc".into());
        let world = World {
            open_questions: vec![memory::QuestionRow::new("q_1", "does x hold", "btc")],
            ..World::default()
        };
        let mut agenda = Vec::new();
        let out = ingest(&charter, &world, &mut agenda);
        assert!(agenda.is_empty());
        assert_eq!(out.skipped[0].why, SkipReason::Excluded);
    }

    #[test]
    fn an_empty_scope_admits_everything_rather_than_nothing() {
        // A charter naming no instruments has not excluded them all. Read the
        // other way round, every subscription in the platform silently drains.
        let charter = charter_over(&[]);
        let world = World {
            open_questions: vec![memory::QuestionRow::new("q_1", "does x hold", "eth")],
            ..World::default()
        };
        let mut agenda = Vec::new();
        let _ = ingest(&charter, &world, &mut agenda);
        assert_eq!(agenda.len(), 1);
    }

    #[test]
    fn the_source_order_is_the_specs_table_order_not_the_iteration_order() {
        // Whichever source creates the item is the first entry in its source
        // list, and the digest shows that one. §4's table is the order.
        let charter = charter_over(&["btc"]);
        let world = World {
            contradictions: vec![memory::ContradictionPair::new("f_a", "f_b", "btc", "why q")],
            open_questions: vec![memory::QuestionRow::new("q_1", "why q", "btc").blocking()],
            ..World::default()
        };
        let mut agenda = Vec::new();
        let _ = ingest(&charter, &world, &mut agenda);
        assert_eq!(agenda[0].sources[0], Source::OpenQuestionBlocking);
    }

    #[test]
    fn not_evaluable_does_not_insert_and_is_not_silent() {
        let charter = charter_over(&["btc"]);
        let world = World {
            negatives: vec![ledger::NegativeRow::new(
                "n_1",
                "btc",
                "momentum needs more data",
                "insufficient_data",
            )
            .measured_to("2026-01-31")],
            // `measured` is empty: nothing told the driver what the coverage is.
            ..World::default()
        };
        let mut agenda = Vec::new();
        let out = ingest(&charter, &world, &mut agenda);
        assert!(agenda.is_empty(), "an unanswerable condition inserts nothing");
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::NotEvaluable);
    }
}

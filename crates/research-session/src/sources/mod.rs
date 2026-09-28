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
//!
//! # The session is named, not inferred from the board
//!
//! [`ingest_for`] takes the session id. The shape before it read the id off
//! `agenda.first()`, which yielded `Uuid::nil()` for an empty agenda and left
//! the caller no choice but to skip all five subscriptions whenever the board
//! held nothing non-terminal. That is the worst possible moment to stop looking:
//! a drained agenda is where §4.9 chooses between replenishing and completing,
//! and it was choosing over a `World` these subscriptions had never been allowed
//! to read. The property that shape bought — a subscription cannot build an item
//! for another session — is kept as a check rather than as a side effect; see
//! [`crate::harvest::SkipReason::NotThisSessionsAgenda`].

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
/// # An empty board is the case this exists for
///
/// The session is a parameter and not read off `agenda.first()`, and that is the
/// fix rather than a tidy-up. Reading it off the agenda answered `Uuid::nil()`
/// for an empty one, so the caller's only defence was to run none of the five
/// subscriptions whenever the board held nothing non-terminal — which turned the
/// session's eyes off at the one moment it had nothing else to do. A drained
/// agenda is where §4.9 decides between replenishing and completing, and it
/// decided that with a `World` nobody had been allowed to ingest.
///
/// What is *not* given away by naming the session: reading it off the agenda
/// also meant a subscription could not build an item for some other session.
/// That is now checked — an agenda carrying an item of another session is
/// refused with [`SkipReason::NotThisSessionsAgenda`] and nothing is ingested,
/// because growing another session's board is worse than ingesting nothing.
#[must_use]
pub fn ingest_for(
    session: uuid::Uuid,
    charter: &Charter,
    world: &World,
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    if let Some(foreign) = foreign_agenda(session, agenda) {
        out.skipped.push(foreign);
        return out;
    }
    out.absorb(memory::open_questions(
        session,
        charter,
        &world.open_questions,
        agenda,
    ));
    out.absorb(ledger::retry_conditions(
        session,
        charter,
        &world.negatives,
        &world.measured,
        agenda,
    ));
    out.absorb(memory::contradictions(
        session,
        charter,
        &world.contradictions,
        agenda,
    ));
    out.absorb(memory::stale_findings(
        session,
        charter,
        &world.stale_findings,
        agenda,
    ));
    out.absorb(observe::observations(
        session,
        charter,
        &world.observations,
        &world.already_raised,
        agenda,
    ));
    out
}

/// [`ingest_for`] with the session taken from the agenda it is given.
///
/// The door that cannot answer for an empty board, kept so a caller that has an
/// agenda and no session id still compiles. It **refuses** rather than ingesting
/// under a nil session: an empty agenda yields one
/// [`SkipReason::SessionNotNamed`] line and nothing else, so the gap is a line
/// in the log rather than five subscriptions that silently did not run.
///
/// Prefer [`ingest_for`]. A caller that knows which session it is driving
/// always does.
#[must_use]
pub fn ingest(charter: &Charter, world: &World, agenda: &mut Vec<AgendaItem>) -> Ingest {
    match memory::session_of(agenda) {
        Some(session) => ingest_for(session, charter, world, agenda),
        None => Ingest {
            inserted: Vec::new(),
            skipped: vec![Skipped {
                what: "sources 4.3-4.7".to_string(),
                why: SkipReason::SessionNotNamed,
            }],
        },
    }
}

/// The refusal for an agenda that is not the named session's.
fn foreign_agenda(session: uuid::Uuid, agenda: &[AgendaItem]) -> Option<Skipped> {
    let other = agenda.iter().find(|i| i.session_id != session)?;
    Some(Skipped {
        what: other.id.to_string(),
        why: SkipReason::NotThisSessionsAgenda,
    })
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
        let out = ingest_for(session(), &charter, &world, &mut agenda);

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
        let out = ingest_for(session(), &charter, &world, &mut agenda);
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
        let out = ingest_for(session(), &charter, &world, &mut agenda);
        assert!(agenda.is_empty());
        assert_eq!(out.skipped.len(), 2, "{:?}", out.skipped);
        assert!(out.skipped.iter().all(|s| s.why == SkipReason::OutOfScope));
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
        let out = ingest_for(session(), &charter, &world, &mut agenda);
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
        let _ = ingest_for(session(), &charter, &world, &mut agenda);
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
        let _ = ingest_for(session(), &charter, &world, &mut agenda);
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
        let out = ingest_for(session(), &charter, &world, &mut agenda);
        assert!(
            agenda.is_empty(),
            "an unanswerable condition inserts nothing"
        );
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::NotEvaluable);
    }

    // ── The session is named, not read off the board ─────────────────────────

    /// A world with something for each of the five subscriptions.
    fn a_busy_world() -> World {
        World {
            open_questions: vec![memory::QuestionRow::new(
                "q_1",
                "does funding predict",
                "btc",
            )],
            negatives: vec![ledger::NegativeRow::new(
                "n_1",
                "btc",
                "momentum needs more data",
                "insufficient_data",
            )
            .measured_to("2026-01-31")],
            contradictions: vec![memory::ContradictionPair::new(
                "f_a",
                "f_b",
                "btc",
                "which way does carry go",
            )],
            stale_findings: vec![
                memory::FindingRow::new("f_1", "btc", "x holds").stale_since("2026-06-19")
            ],
            measured: ledger::Measured::default(),
            observations: vec![observe::Observation::RegimeFlip {
                subject: "btc".into(),
                from: "calm".into(),
                to: "stressed".into(),
                since: "2026-09-01".into(),
            }],
            already_raised: observe::Raised::default(),
        }
    }

    /// The bug this parameter exists for: a board with nothing on it.
    ///
    /// Before the session was named, every item's `session_id` came from
    /// `agenda.first()`, so an empty board produced `Uuid::nil()` and the only
    /// caller had to skip all five subscriptions to keep those rows out of a
    /// `NOT NULL` column. A drained agenda is where §4.9 chooses between
    /// replenishing and completing, so the session stopped watching the world at
    /// the one moment it had nothing else to do.
    #[test]
    fn a_drained_board_still_lets_the_subscriptions_insert_and_never_under_a_nil_session() {
        let charter = charter_over(&["btc"]);
        let world = a_busy_world();
        let mut agenda = Vec::new();
        let out = ingest_for(session(), &charter, &world, &mut agenda);

        assert!(
            !agenda.is_empty(),
            "an empty board is the case the subscriptions are for, not the case they refuse"
        );
        assert!(
            out.skipped
                .iter()
                .all(|s| s.why != SkipReason::SessionNotNamed),
            "nothing was skipped for want of a session: {:?}",
            out.skipped
        );
        for item in &agenda {
            assert_eq!(
                item.session_id,
                session(),
                "an item built for the session it was given, never for the nil one a \
                 `NOT NULL` column would refuse: {item:?}"
            );
        }
    }

    /// The property naming the session would otherwise have given away.
    #[test]
    fn no_subscription_can_grow_the_board_of_a_session_it_was_not_given() {
        let charter = charter_over(&["btc"]);
        let world = a_busy_world();
        let someone_else = Uuid::from_u128(0x5e55_1042);
        let mut agenda = vec![AgendaItem::new(
            someone_else,
            "a question on another session's board",
            "btc",
            Source::CharterSeed,
        )];
        let out = ingest_for(session(), &charter, &world, &mut agenda);

        assert_eq!(agenda.len(), 1, "nothing was added: {agenda:?}");
        assert_eq!(out.inserted.len(), 0);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].why, SkipReason::NotThisSessionsAgenda);
    }

    /// The derived door says so rather than keying on a nil.
    #[test]
    fn the_agenda_derived_door_refuses_an_empty_board_rather_than_ingesting_under_a_nil_session() {
        let charter = charter_over(&["btc"]);
        let world = a_busy_world();
        let mut agenda = Vec::new();
        let out = ingest(&charter, &world, &mut agenda);

        assert!(agenda.is_empty(), "nothing was built: {agenda:?}");
        assert_eq!(out.inserted.len(), 0);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(
            out.skipped[0].why,
            SkipReason::SessionNotNamed,
            "the gap is a word on the log, not five subscriptions that quietly did not run"
        );
        assert_ne!(
            SkipReason::SessionNotNamed.as_str(),
            SkipReason::NotEvaluable.as_str(),
            "an unnamed session is not an unanswerable condition"
        );
    }

    #[test]
    fn the_board_names_a_session_only_when_it_has_one() {
        assert_eq!(memory::session_of(&[]), None);
        assert_eq!(
            memory::session_of(&[AgendaItem::new(session(), "q", "btc", Source::User)]),
            Some(session())
        );
    }
}

//! Sources 4.3, 4.5 and 4.6: the three subscriptions whose whole operand is a
//! memory row (AGENT-014 §4.3, §4.5, §4.6).
//!
//! An open question, a contradiction pair and a finding past its `valid_until`
//! all have this in common: the platform already decided the thing. A question
//! is `open` because no finding has cited it; a pair conflicts because
//! `harness::memory::conflict` computed that it does; a finding is stale
//! because `Entry::valid_until` passed. **None of the three rules is re-derived
//! here**, and that is the point — a second opinion about whether two findings
//! contradict is a second answer with no way to say which is right.
//!
//! What this module decides is the only thing left: *which workflow, under
//! which source, with which sentence, and does the charter admit it.*
//!
//! # The rows are shapes, not a second schema
//!
//! [`QuestionRow`], [`ContradictionPair`] and [`FindingRow`] carry what the
//! rule reads and nothing else. They are not a copy of
//! `harness::memory::Entry` — an `Entry` has twenty fields, eighteen of which
//! no rule here looks at, and a struct that carries them would be a second
//! definition of memory that drifts. The driver projects; this decides.

use uuid::Uuid;

use super::{admitted, offer_all, Ingest};
use crate::agenda::{AgendaItem, Source};
use crate::charter::Charter;

/// An open question, as source 4.3 reads it.
///
/// Mirrors the fields of `harness::memory::questions::OpenQuestion` that the
/// rule uses. `status` is **not** among them: §4.3 says "`status = open`", and
/// a closed question is not something this module filters out — it is something
/// the driver's query never returns. A struct with a `status` field invites a
/// caller to pass a closed one and trust the filter, and the filter that
/// matters is an index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRow {
    /// The memory id (`q_…`), which becomes the item's `source_ref`.
    pub id: String,
    pub question: String,
    pub subject: String,
    /// §4.3: blocking first. Carried as a heavier [`Source`] rather than as a
    /// flag, so the scorer needs to know nothing about questions.
    pub blocking: bool,
}

impl QuestionRow {
    #[must_use]
    pub fn new(id: &str, question: &str, subject: &str) -> Self {
        Self {
            id: id.to_string(),
            question: question.to_string(),
            subject: subject.to_string(),
            blocking: false,
        }
    }

    #[must_use]
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }

    /// The source this question comes in under (§5.1's weight table: a blocking
    /// question is 1.3 and an ordinary one 1.0).
    #[must_use]
    pub fn source(&self) -> Source {
        if self.blocking {
            Source::OpenQuestionBlocking
        } else {
            Source::OpenQuestion
        }
    }
}

/// Source 4.3 — open questions in scope, blocking first.
///
/// Sorting happens **here** rather than being left to the scorer, because §4.3
/// says "blocking first" about the order they are ingested in and two items
/// created in the same iteration break their score tie on `created_at`
/// (§5.4). An unsorted ingest makes that tie-break random.
#[must_use]
pub fn open_questions(
    charter: &Charter,
    rows: &[QuestionRow],
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    let mut rows: Vec<&QuestionRow> = rows.iter().collect();
    // Stable, so two blocking questions keep the order the store returned them
    // in rather than being reshuffled by the sort.
    rows.sort_by_key(|r| !r.blocking);

    let mut candidates = Vec::new();
    for r in rows {
        if r.question.trim().is_empty() {
            out.skipped.push(crate::harvest::Skipped {
                what: r.id.clone(),
                why: crate::harvest::SkipReason::Blank,
            });
            continue;
        }
        match admitted(charter, &r.subject, &r.id) {
            Err(s) => out.skipped.push(s),
            Ok(()) => candidates.push(
                AgendaItem::new(session_of(agenda), &r.question, &r.subject, r.source())
                    .with_source_ref(&r.id),
            ),
        }
    }
    out.inserted = offer_all(agenda, candidates);
    out
}

/// Two findings the platform has already decided conflict (AGENT-007 §2.2.1).
///
/// The pair is unordered as far as this module is concerned and both ids go on
/// the item, because the agenda card has to open **both** — a diagnosis naming
/// one half of a contradiction is a card a human cannot act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContradictionPair {
    pub left: String,
    pub right: String,
    pub subject: String,
    /// What the run should be asked. Supplied rather than composed, because the
    /// two claims are the readable version of the conflict and this module does
    /// not have them.
    pub question: String,
}

impl ContradictionPair {
    #[must_use]
    pub fn new(left: &str, right: &str, subject: &str, question: &str) -> Self {
        Self {
            left: left.to_string(),
            right: right.to_string(),
            subject: subject.to_string(),
            question: question.to_string(),
        }
    }

    /// The pair's identity, order-independent.
    ///
    /// `f_a`/`f_b` and `f_b`/`f_a` are one contradiction, and a store that
    /// returns the pair from either side on different iterations would
    /// otherwise raise two items whose dedupe keys happen to match only because
    /// the question text does.
    #[must_use]
    pub fn key(&self) -> String {
        let (a, b) = if self.left <= self.right {
            (&self.left, &self.right)
        } else {
            (&self.right, &self.left)
        };
        format!("{a}/{b}")
    }
}

/// Source 4.5 — an unresolved contradiction pair becomes a `diagnose`.
///
/// §4.5's own gloss: *"resolution is a research act"*. It is the heaviest source
/// in §5.1's table at 1.5, because a contradiction is not a gap in the
/// knowledge base — it is the knowledge base being wrong, and every run that
/// reads the brief reads both halves until somebody settles it.
#[must_use]
pub fn contradictions(
    charter: &Charter,
    pairs: &[ContradictionPair],
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    let mut candidates = Vec::new();
    for p in pairs {
        if p.question.trim().is_empty() {
            out.skipped.push(crate::harvest::Skipped {
                what: p.key(),
                why: crate::harvest::SkipReason::Blank,
            });
            continue;
        }
        match admitted(charter, &p.subject, &p.key()) {
            Err(s) => out.skipped.push(s),
            Ok(()) => candidates.push(
                AgendaItem::new(
                    session_of(agenda),
                    &p.question,
                    &p.subject,
                    Source::Contradiction,
                )
                .with_source_ref(&p.left)
                .with_source_ref(&p.right),
            ),
        }
    }
    out.inserted = offer_all(agenda, candidates);
    out
}

/// An `established` finding past its `valid_until`, as source 4.6 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRow {
    pub id: String,
    pub subject: String,
    /// The claim, used to phrase the replication question.
    pub claim: String,
    /// ISO date the entry stopped being fresh, for the item's sentence. The
    /// **comparison** is the driver's: `valid_until < today` is an index
    /// predicate, and a second copy of it here would be a second answer to a
    /// question the query already answered.
    pub valid_until: Option<String>,
}

impl FindingRow {
    #[must_use]
    pub fn new(id: &str, subject: &str, claim: &str) -> Self {
        Self {
            id: id.to_string(),
            subject: subject.to_string(),
            claim: claim.to_string(),
            valid_until: None,
        }
    }

    #[must_use]
    pub fn stale_since(mut self, date: &str) -> Self {
        self.valid_until = Some(date.to_string());
        self
    }

    /// The question a replication asks.
    ///
    /// Phrased as a question about whether the claim **still** holds, not about
    /// whether it holds: the first is a replication and the second is the
    /// original study. A `replicate` run handed the second would preregister
    /// against a fresh window and lose the comparison that is the whole point.
    #[must_use]
    pub fn question(&self) -> String {
        match &self.valid_until {
            Some(d) => format!("does this still hold, last verified to {d}: {}", self.claim),
            // Never "since unknown" and never a date invented from today: a
            // sentence naming a date nobody measured is the shape this crate
            // spends most of its type system refusing.
            None => format!("does this still hold: {}", self.claim),
        }
    }
}

/// Source 4.6 — a stale `established` finding becomes a `replicate`.
///
/// This is the re-verification AGENT-007 P7 promises and that nothing had ever
/// scheduled: `Kind::validity_days` has set a 90-day expiry on every finding
/// since it was written, and until a session cared, a finding passing it just
/// stopped appearing in briefs. The session that depends on the finding is the
/// thing that has a reason to pay for checking it.
#[must_use]
pub fn stale_findings(
    charter: &Charter,
    rows: &[FindingRow],
    agenda: &mut Vec<AgendaItem>,
) -> Ingest {
    let mut out = Ingest::default();
    let mut candidates = Vec::new();
    for r in rows {
        if r.claim.trim().is_empty() {
            out.skipped.push(crate::harvest::Skipped {
                what: r.id.clone(),
                why: crate::harvest::SkipReason::Blank,
            });
            continue;
        }
        match admitted(charter, &r.subject, &r.id) {
            Err(s) => out.skipped.push(s),
            Ok(()) => candidates.push(
                AgendaItem::new(
                    session_of(agenda),
                    &r.question(),
                    &r.subject,
                    Source::StaleFinding,
                )
                .with_source_ref(&r.id),
            ),
        }
    }
    out.inserted = offer_all(agenda, candidates);
    out
}

/// The session every item on this agenda belongs to.
///
/// Read off the agenda rather than passed, so a subscription cannot create an
/// item for a session other than the one whose agenda it is looking at. An
/// empty agenda has no session to read and yields nil — the driver writes the
/// row and the `session_id` column is `NOT NULL` with a foreign key, so a nil
/// reaching the database is refused there rather than stored.
pub(crate) fn session_of(agenda: &[AgendaItem]) -> Uuid {
    agenda.first().map_or_else(Uuid::nil, |i| i.session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::ItemStatus;
    use crate::sources::tests::{charter_over, session};

    fn seeded(agenda_of: Uuid) -> Vec<AgendaItem> {
        vec![AgendaItem {
            status: ItemStatus::Done {
                verdict: "accept".into(),
            },
            ..AgendaItem::new(agenda_of, "an unrelated settled question", "", Source::User)
        }]
    }

    #[test]
    fn a_blocking_question_is_ingested_before_an_ordinary_one() {
        let charter = charter_over(&["btc"]);
        let rows = vec![
            QuestionRow::new("q_plain", "the ordinary one", "btc"),
            QuestionRow::new("q_block", "the blocking one", "btc").blocking(),
        ];
        let mut agenda = seeded(session());
        let _ = open_questions(&charter, &rows, &mut agenda);
        assert_eq!(agenda[1].source_refs, vec!["q_block".to_string()]);
        assert_eq!(agenda[2].source_refs, vec!["q_plain".to_string()]);
    }

    #[test]
    fn two_blocking_questions_keep_the_stores_order() {
        let charter = charter_over(&["btc"]);
        let rows = vec![
            QuestionRow::new("q_1", "first", "btc").blocking(),
            QuestionRow::new("q_2", "second", "btc").blocking(),
        ];
        let mut agenda = seeded(session());
        let _ = open_questions(&charter, &rows, &mut agenda);
        assert_eq!(agenda[1].source_refs, vec!["q_1".to_string()]);
        assert_eq!(agenda[2].source_refs, vec!["q_2".to_string()]);
    }

    #[test]
    fn a_question_carries_the_workflow_its_source_asks_for() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded(session());
        let _ = open_questions(
            &charter,
            &[QuestionRow::new("q_1", "does x hold", "btc")],
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, "investigate", "§4.3's default");
    }

    #[test]
    fn a_contradiction_opens_both_halves_and_asks_for_a_diagnosis() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded(session());
        let _ = contradictions(
            &charter,
            &[ContradictionPair::new(
                "f_47c",
                "f_31a",
                "btc",
                "which of these two holds",
            )],
            &mut agenda,
        );
        assert_eq!(
            agenda[1].workflow, "diagnose",
            "§4.5: resolution is research"
        );
        assert_eq!(
            agenda[1].source_refs,
            vec!["f_47c".to_string(), "f_31a".to_string()],
            "a card that opens one half of a contradiction cannot be acted on"
        );
    }

    #[test]
    fn a_pair_has_one_identity_whichever_side_the_store_returns_it_from() {
        let a = ContradictionPair::new("f_a", "f_b", "btc", "q");
        let b = ContradictionPair::new("f_b", "f_a", "btc", "q");
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn a_contradiction_outweighs_a_follow_up_because_it_is_the_base_being_wrong() {
        assert!(Source::Contradiction.weight() > Source::RefinementFollowup.weight());
        assert!(Source::Contradiction.weight() > Source::OpenQuestionBlocking.weight());
    }

    #[test]
    fn a_stale_finding_asks_whether_it_still_holds_rather_than_whether_it_holds() {
        let f = FindingRow::new("f_31a", "btc", "funding predicts 1h returns")
            .stale_since("2026-06-19");
        let q = f.question();
        assert!(q.contains("still hold"), "{q}");
        assert!(q.contains("2026-06-19"), "{q}");
    }

    #[test]
    fn a_finding_with_no_valid_until_names_no_date_rather_than_inventing_one() {
        let f = FindingRow::new("f_1", "btc", "x holds");
        let q = f.question();
        assert!(q.contains("still hold"), "{q}");
        assert!(
            !q.contains("unknown") && !q.contains("None"),
            "a sentence naming a date nobody measured: {q}"
        );
    }

    #[test]
    fn a_stale_finding_asks_for_a_replicate() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded(session());
        let _ = stale_findings(
            &charter,
            &[FindingRow::new("f_1", "btc", "x holds").stale_since("2026-06-19")],
            &mut agenda,
        );
        assert_eq!(agenda[1].workflow, "replicate");
        assert_eq!(agenda[1].sources, vec![Source::StaleFinding]);
    }

    #[test]
    fn an_item_belongs_to_the_session_whose_agenda_it_was_offered_to() {
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded(session());
        let _ = open_questions(
            &charter,
            &[QuestionRow::new("q_1", "does x hold", "btc")],
            &mut agenda,
        );
        assert_eq!(agenda[1].session_id, session());
    }

    #[test]
    fn a_blank_question_is_refused_rather_than_keyed_on_nothing() {
        // Every blank sentence normalises to the same dedupe key, so the first
        // one swallows every later one from any source.
        let charter = charter_over(&["btc"]);
        let mut agenda = seeded(session());
        let out = open_questions(
            &charter,
            &[QuestionRow::new("q_1", "   ", "btc")],
            &mut agenda,
        );
        assert_eq!(agenda.len(), 1);
        assert_eq!(out.skipped[0].why, crate::harvest::SkipReason::Blank);
    }
}

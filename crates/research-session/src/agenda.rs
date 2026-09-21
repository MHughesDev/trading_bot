//! The agenda: the items a session may claim, where they came from, and the
//! first of the three places duplicate work is refused (AGENT-014 §2.3, §4).
//!
//! # The dedupe key is normalisation, not similarity
//!
//! [`dedupe_key`] lower-cases, strips punctuation and collapses whitespace. It
//! does **not** try to decide that two differently worded questions are the same
//! question — that is [`crate::agenda::Source`]-independent judgement and it
//! belongs to `hypothesis_precheck`, which is layer two and which has the ledger
//! in front of it. Layer one is cheap and exact on purpose: it catches the case
//! that actually happens, which is two subscriptions proposing the *same
//! sentence* on the same iteration, and it never merges two questions a human
//! would want asked separately.
//!
//! The layer that caught a duplicate is recorded, because P3 says the scorecard
//! reports which one did.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where an agenda item came from (AGENT-014 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// 4.1 — written into the charter at start.
    CharterSeed,
    /// 4.2 — the human, any time. Bypasses scoring.
    User,
    /// 4.3 — an open question in scope.
    OpenQuestion,
    /// 4.3 — an open question in scope marked `blocking`.
    OpenQuestionBlocking,
    /// 4.4 — a negative result whose `retry_condition` now evaluates true.
    RetryConditionMet,
    /// 4.5 — a contradiction pair with no resolving finding.
    Contradiction,
    /// 4.6 — an `established` finding past its `valid_until`.
    StaleFinding,
    /// 4.7 — a regime flip, new coverage, or a monitor event.
    Observation,
    /// 4.8 — a finished run's next move worth pursuing.
    RefinementFollowup,
    /// A proposal a finished run made for what to ask next.
    ProposalFromRun,
    /// 4.9 — the survey run's proposed questions, when the agenda drained.
    Replenish,
}

impl Source {
    pub const ALL: &'static [Source] = &[
        Source::CharterSeed,
        Source::User,
        Source::OpenQuestion,
        Source::OpenQuestionBlocking,
        Source::RetryConditionMet,
        Source::Contradiction,
        Source::StaleFinding,
        Source::Observation,
        Source::RefinementFollowup,
        Source::ProposalFromRun,
        Source::Replenish,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Source::CharterSeed => "charter_seed",
            Source::User => "user",
            Source::OpenQuestion => "open_question",
            Source::OpenQuestionBlocking => "open_question_blocking",
            Source::RetryConditionMet => "retry_condition_met",
            Source::Contradiction => "contradiction",
            Source::StaleFinding => "stale_finding",
            Source::Observation => "observation",
            Source::RefinementFollowup => "refinement_followup",
            Source::ProposalFromRun => "proposal_from_run",
            Source::Replenish => "replenish",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|x| x.as_str() == s)
    }

    /// The source weight in the `value` signal (AGENT-014 §5.1's table).
    ///
    /// `user` is listed as ∞ in the spec, and this returns a finite number for
    /// it because the ∞ is implemented by [`Self::bypasses_scoring`] rather than
    /// by a float that would poison every percentile normalisation it touches.
    /// A weight that makes every other item's normalised value zero is not the
    /// same as an item that is not scored, and the spec means the second.
    #[must_use]
    pub fn weight(self) -> f64 {
        match self {
            Source::User => 1.0,
            Source::Contradiction => 1.5,
            Source::RetryConditionMet | Source::OpenQuestionBlocking => 1.3,
            Source::Observation => 1.2,
            Source::StaleFinding | Source::CharterSeed | Source::OpenQuestion => 1.0,
            Source::RefinementFollowup => 0.8,
            // §4.9: the survey estimates its own value, and the estimate rides
            // on the item rather than on the source.
            Source::Replenish | Source::ProposalFromRun => 1.0,
        }
    }

    /// Whether an item from this source is claimed next regardless of score.
    #[must_use]
    pub fn bypasses_scoring(self) -> bool {
        matches!(self, Source::User)
    }

    /// The workflow the source asks for when the item does not name one
    /// (AGENT-014 §4's table).
    #[must_use]
    pub fn default_workflow(self) -> &'static str {
        match self {
            // Resolution is a research act, not a re-read.
            Source::Contradiction | Source::Observation => "diagnose",
            // The re-verification AGENT-007 P7 promises, scheduled by the
            // session that cares about the finding.
            Source::StaleFinding => "replicate",
            _ => "investigate",
        }
    }
}

/// Where an item is in its life (AGENT-014 §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemStatus {
    Proposed,
    Approved,
    Queued,
    Claimed { run_id: Uuid },
    Running { run_id: Uuid },
    Done { verdict: String },
    Vetoed { by: String },
    Superseded { by_item: Uuid },
    Expired { why: String },
}

impl ItemStatus {
    /// The status word the column holds.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            ItemStatus::Proposed => "proposed",
            ItemStatus::Approved => "approved",
            ItemStatus::Queued => "queued",
            ItemStatus::Claimed { .. } => "claimed",
            ItemStatus::Running { .. } => "running",
            ItemStatus::Done { .. } => "done",
            ItemStatus::Vetoed { .. } => "vetoed",
            ItemStatus::Superseded { .. } => "superseded",
            ItemStatus::Expired { .. } => "expired",
        }
    }

    /// Every status word, in the order the board shows them.
    pub const WORDS: &'static [&'static str] = &[
        "proposed",
        "approved",
        "queued",
        "claimed",
        "running",
        "done",
        "vetoed",
        "superseded",
        "expired",
    ];

    /// The words a query means by "finished".
    ///
    /// Written out because SQL cannot call [`Self::is_terminal`] and the partial
    /// unique index that enforces layer one needs the list. Held equal to the
    /// method by `the_terminal_words_are_the_terminal_statuses` below, and the
    /// migration's copy is held equal to *this* by
    /// `crates/invariants/tests/a_session_transition_table_has_one_definition.rs`
    /// — so the rule is written three times and asserted twice, rather than
    /// written three times and hoped about.
    pub const TERMINAL_WORDS: &'static [&'static str] =
        &["done", "vetoed", "superseded", "expired"];

    /// Which of the board's five columns this status falls in (AGENT-014 §9).
    ///
    /// Nine statuses, five columns, and the mapping is a **projection** rather
    /// than a second vocabulary: the card still shows the true status word, and
    /// this only decides which pile it sits in. `superseded` and `expired` are
    /// both "finished without reaching a verdict" and go in `done` beside the
    /// ones that did, because the alternative — two more columns a human has to
    /// read every time — buys nothing the badge does not already say.
    ///
    /// Held total by `every_status_falls_in_one_of_the_five_columns`.
    #[must_use]
    pub fn board_column(&self) -> &'static str {
        match self {
            ItemStatus::Proposed => "proposed",
            ItemStatus::Approved | ItemStatus::Queued => "queued",
            ItemStatus::Claimed { .. } | ItemStatus::Running { .. } => "running",
            ItemStatus::Done { .. } | ItemStatus::Superseded { .. } | ItemStatus::Expired { .. } => {
                "done"
            }
            ItemStatus::Vetoed { .. } => "vetoed",
        }
    }

    /// The board's columns, left to right.
    pub const COLUMNS: &'static [&'static str] =
        &["proposed", "queued", "running", "done", "vetoed"];

    /// Whether this item can still change.
    ///
    /// Dedupe reads this: an insert whose key matches a **non-terminal** item
    /// extends that item's sources instead of inserting. A key matching a `done`
    /// item does insert, because asking a finished question again is
    /// replication, and replication is something the platform is for.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ItemStatus::Done { .. }
                | ItemStatus::Vetoed { .. }
                | ItemStatus::Superseded { .. }
                | ItemStatus::Expired { .. }
        )
    }

    /// Whether the driver may claim this item now.
    #[must_use]
    pub fn is_claimable(&self) -> bool {
        matches!(self, ItemStatus::Approved | ItemStatus::Queued)
    }

    /// The run this item is attached to, if any.
    #[must_use]
    pub fn run_id(&self) -> Option<Uuid> {
        match self {
            ItemStatus::Claimed { run_id } | ItemStatus::Running { run_id } => Some(*run_id),
            _ => None,
        }
    }
}

/// The four signals and their product (AGENT-014 §5.1).
///
/// Held as a struct rather than one number because the decision row records the
/// full candidate set *with its signals*, and a total with no components is not
/// something a human can disagree with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub value: f64,
    pub novelty: f64,
    pub feasibility: f64,
    pub cost: f64,
    pub total: f64,
}

/// The cost floor (AGENT-014 §5.1): a workflow that looks free still is not.
pub const COST_FLOOR_USD: f64 = 0.10;

impl Score {
    /// `total = value × novelty × feasibility / cost`, with the cost floor.
    #[must_use]
    pub fn combine(value: f64, novelty: f64, feasibility: f64, cost: f64) -> Self {
        let cost = cost.max(COST_FLOOR_USD);
        Self {
            value,
            novelty,
            feasibility,
            cost,
            total: value * novelty * feasibility / cost,
        }
    }
}

/// One thing the session may go and ask (AGENT-014 §2.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgendaItem {
    pub id: Uuid,
    pub session_id: Uuid,
    pub question: String,
    /// What the question is about; half of the dedupe key and the subject the
    /// objective is weighed against.
    #[serde(default)]
    pub subject: String,
    pub workflow: String,
    /// Every source that wanted this question. A question three sources want
    /// outranks one that one source wants, which is why this is a list and not
    /// a field that the last writer overwrites.
    pub sources: Vec<Source>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub dedupe_key: String,
    #[serde(default)]
    pub score: Option<Score>,
    pub status: ItemStatus,
    #[serde(default)]
    pub depends_on: Vec<Uuid>,
    /// A child that ends `failed`, `fenced` or `interrupted` re-queues the item
    /// with this raised; at [`MAX_ATTEMPTS`] the item expires.
    #[serde(default)]
    pub attempts: i32,
    #[serde(default)]
    pub yield_ref: Option<String>,
}

/// An item is retried once and then expires (AGENT-014 §6.6).
pub const MAX_ATTEMPTS: i32 = 2;

impl AgendaItem {
    /// A fresh item, keyed from its own question and subject.
    #[must_use]
    pub fn new(session_id: Uuid, question: &str, subject: &str, source: Source) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            question: question.to_string(),
            subject: subject.to_string(),
            workflow: source.default_workflow().to_string(),
            sources: vec![source],
            source_refs: Vec::new(),
            dedupe_key: dedupe_key(question, subject),
            score: None,
            status: ItemStatus::Proposed,
            depends_on: Vec::new(),
            attempts: 0,
            yield_ref: None,
        }
    }

    #[must_use]
    pub fn with_workflow(mut self, workflow: &str) -> Self {
        self.workflow = workflow.to_string();
        self
    }

    #[must_use]
    pub fn with_source_ref(mut self, r: &str) -> Self {
        self.source_refs.push(r.to_string());
        self
    }

    /// The highest source weight among everything that wanted this item.
    ///
    /// Max rather than sum: three sources wanting a question is evidence it
    /// matters, but adding weights would let a question wanted weakly by five
    /// subscriptions outrank a contradiction, and a contradiction is a fact
    /// about the knowledge base being wrong.
    #[must_use]
    pub fn source_weight(&self) -> f64 {
        self.sources
            .iter()
            .map(|s| s.weight())
            .fold(0.0_f64, f64::max)
    }

    /// Whether this item skips the scorer entirely.
    #[must_use]
    pub fn bypasses_scoring(&self) -> bool {
        self.sources.iter().any(|s| s.bypasses_scoring())
    }
}

/// The normalised key two inserts are compared on (AGENT-014 §4).
#[must_use]
pub fn dedupe_key(question: &str, subject: &str) -> String {
    let normalise = |s: &str| -> String {
        let mut out = String::with_capacity(s.len());
        let mut last_space = true;
        for ch in s.chars() {
            if ch.is_alphanumeric() {
                for l in ch.to_lowercase() {
                    out.push(l);
                }
                last_space = false;
            } else if !last_space {
                out.push(' ');
                last_space = true;
            }
        }
        out.trim_end().to_string()
    };
    format!("{}|{}", normalise(subject), normalise(question))
}

/// Which of the three layers refused a duplicate (AGENT-014 P3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DedupeLayer {
    /// The key matched a non-terminal item; that item's sources were extended.
    Key,
    /// `hypothesis_precheck` said `reject_duplicate` at intake.
    Intake,
    /// The family's trial count in the ledger drove `novelty` to zero.
    Ledger,
}

impl DedupeLayer {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DedupeLayer::Key => "key",
            DedupeLayer::Intake => "intake",
            DedupeLayer::Ledger => "ledger",
        }
    }
}

/// What happened when an item was offered to the agenda.
#[derive(Debug, Clone, PartialEq)]
pub enum Insert {
    /// A new item.
    Added(Box<AgendaItem>),
    /// The key matched; the existing item now carries one more source.
    Merged { into: Uuid, source: Source },
}

/// Layer one: offer an item to an agenda.
///
/// Returns [`Insert::Merged`] when the key matches any **non-terminal** item,
/// after extending that item's source list. The agenda is passed as a mutable
/// slice rather than read from the database so that the rule is testable without
/// one, and so that the driver applies it in the same transaction that inserts.
pub fn offer(agenda: &mut [AgendaItem], item: AgendaItem) -> Insert {
    if let Some(existing) = agenda
        .iter_mut()
        .find(|e| e.dedupe_key == item.dedupe_key && !e.status.is_terminal())
    {
        let source = item.sources.first().copied().unwrap_or(Source::CharterSeed);
        if !existing.sources.contains(&source) {
            existing.sources.push(source);
        }
        for r in item.source_refs {
            if !existing.source_refs.contains(&r) {
                existing.source_refs.push(r);
            }
        }
        // The score depended on the source list, so it is no longer the score of
        // this item. Cleared rather than recomputed here: §5.4 says a score is
        // recomputed when a source changes it, and this is that moment, but the
        // scorer needs the whole agenda to normalise against.
        existing.score = None;
        return Insert::Merged {
            into: existing.id,
            source,
        };
    }
    Insert::Added(Box::new(item))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Uuid {
        Uuid::nil()
    }

    #[test]
    fn the_key_ignores_wording_noise_but_not_wording() {
        assert_eq!(
            dedupe_key("Does momentum survive costs?", "BTC-USD"),
            dedupe_key("does  momentum survive costs", "btc usd")
        );
        assert_ne!(
            dedupe_key("does momentum survive costs", "BTC-USD"),
            dedupe_key("does carry survive costs", "BTC-USD")
        );
        // Same question, different instrument, is a different question.
        assert_ne!(
            dedupe_key("does momentum survive costs", "BTC-USD"),
            dedupe_key("does momentum survive costs", "ETH-USD")
        );
    }

    #[test]
    fn a_second_source_extends_the_first_item_instead_of_inserting() {
        let mut agenda = vec![AgendaItem::new(
            session(),
            "does momentum survive costs?",
            "BTC-USD",
            Source::CharterSeed,
        )];
        let again = AgendaItem::new(
            session(),
            "Does momentum survive costs",
            "btc-usd",
            Source::Contradiction,
        )
        .with_source_ref("f_31a/f_47c");

        match offer(&mut agenda, again) {
            Insert::Merged { into, source } => {
                assert_eq!(into, agenda[0].id);
                assert_eq!(source, Source::Contradiction);
            }
            Insert::Added(_) => panic!("the key matched and the item was inserted anyway"),
        }
        assert_eq!(agenda[0].sources.len(), 2);
        assert_eq!(agenda[0].source_refs, vec!["f_31a/f_47c".to_string()]);
        // Two sources wanted it, so it now carries the higher weight.
        assert!((agenda[0].source_weight() - 1.5).abs() < 1e-12);
    }

    #[test]
    fn a_finished_question_can_be_asked_again() {
        let mut agenda = vec![AgendaItem::new(session(), "q", "s", Source::CharterSeed)];
        agenda[0].status = ItemStatus::Done {
            verdict: "refuted".into(),
        };
        let again = AgendaItem::new(session(), "q", "s", Source::StaleFinding);
        assert!(matches!(offer(&mut agenda, again), Insert::Added(_)));
    }

    #[test]
    fn user_is_the_only_source_that_skips_the_scorer() {
        for s in Source::ALL {
            assert_eq!(s.bypasses_scoring(), *s == Source::User, "{}", s.as_str());
        }
        assert!(AgendaItem::new(session(), "q", "s", Source::User).bypasses_scoring());
    }

    #[test]
    fn every_source_word_round_trips() {
        for s in Source::ALL {
            assert_eq!(Source::parse(s.as_str()), Some(*s));
        }
    }

    #[test]
    fn the_cost_floor_stops_a_free_looking_workflow_dominating() {
        let cheap = Score::combine(1.0, 1.0, 1.0, 0.0);
        assert!((cheap.cost - COST_FLOOR_USD).abs() < 1e-12);
        assert!(cheap.total.is_finite());
        // Infeasible is zero however cheap it is.
        assert!((Score::combine(9.0, 1.0, 0.0, 0.1).total).abs() < 1e-12);
    }

    #[test]
    fn every_status_falls_in_one_of_the_five_columns() {
        let r = Uuid::new_v4();
        let every = [
            ItemStatus::Proposed,
            ItemStatus::Approved,
            ItemStatus::Queued,
            ItemStatus::Claimed { run_id: r },
            ItemStatus::Running { run_id: r },
            ItemStatus::Done { verdict: "x".into() },
            ItemStatus::Vetoed { by: "x".into() },
            ItemStatus::Superseded { by_item: r },
            ItemStatus::Expired { why: "x".into() },
        ];
        for s in &every {
            assert!(
                ItemStatus::COLUMNS.contains(&s.board_column()),
                "{} lands in `{}`, which is not a column the board has",
                s.word(),
                s.board_column()
            );
        }
        // And every column is reachable: a column nothing can be in is a column
        // that renders empty forever and teaches the reader it is broken.
        for col in ItemStatus::COLUMNS {
            assert!(
                every.iter().any(|s| s.board_column() == *col),
                "nothing can ever be in the `{col}` column"
            );
        }
    }

    #[test]
    fn the_terminal_words_are_the_terminal_statuses() {
        let r = Uuid::new_v4();
        let every = [
            ItemStatus::Proposed,
            ItemStatus::Approved,
            ItemStatus::Queued,
            ItemStatus::Claimed { run_id: r },
            ItemStatus::Running { run_id: r },
            ItemStatus::Done { verdict: "x".into() },
            ItemStatus::Vetoed { by: "x".into() },
            ItemStatus::Superseded { by_item: r },
            ItemStatus::Expired { why: "x".into() },
        ];
        let derived: Vec<&str> = every
            .iter()
            .filter(|s| s.is_terminal())
            .map(ItemStatus::word)
            .collect();
        assert_eq!(
            derived,
            ItemStatus::TERMINAL_WORDS,
            "the list a query filters on and the method the code branches on disagree, so a \
             finished item is finished in one place and live in the other"
        );
    }

    #[test]
    fn a_terminal_status_is_terminal_and_a_claimable_one_is_not() {
        assert!(ItemStatus::Done { verdict: "x".into() }.is_terminal());
        assert!(ItemStatus::Vetoed { by: "mason".into() }.is_terminal());
        assert!(!ItemStatus::Queued.is_terminal());
        assert!(ItemStatus::Queued.is_claimable());
        assert!(ItemStatus::Approved.is_claimable());
        assert!(!ItemStatus::Proposed.is_claimable());
        let r = Uuid::new_v4();
        assert_eq!(ItemStatus::Running { run_id: r }.run_id(), Some(r));
    }

    #[test]
    fn the_status_word_list_covers_every_variant() {
        let r = Uuid::new_v4();
        let all = [
            ItemStatus::Proposed,
            ItemStatus::Approved,
            ItemStatus::Queued,
            ItemStatus::Claimed { run_id: r },
            ItemStatus::Running { run_id: r },
            ItemStatus::Done { verdict: "x".into() },
            ItemStatus::Vetoed { by: "x".into() },
            ItemStatus::Superseded { by_item: r },
            ItemStatus::Expired { why: "x".into() },
        ];
        let words: Vec<&str> = all.iter().map(ItemStatus::word).collect();
        assert_eq!(words, ItemStatus::WORDS);
    }
}

//! The run context store (AGENT-005 §14; Set S task S-9.1).
//!
//! # What changes
//!
//! Part I holds observations in `Loop::observations`, a `Vec<Block>` that grows
//! and gets trimmed when the window fills. A trimmed block is *gone*: there is no
//! id to ask about, no record that it existed, and nothing to bring back. Every
//! mechanism Part II adds — the manifest, `expand`, utility attribution,
//! counterfactual ablation — needs the observation to still exist after it has
//! left the prompt.
//!
//! So the buffer becomes a store of addressable typed [`Record`]s, and the rule
//! is the one §14 states plainly: **records are never deleted during a run**.
//! They leave the prompt, never the store. That single property is what turns
//! every later loss from lossy into recoverable.
//!
//! # What does *not* become a record
//!
//! §14's sentence is "every **observation** becomes an addressable record", and
//! that is the line this module draws. The charter and the state card are not
//! observations: they are rewritten from state on every render, they do not
//! accumulate, and they have no step they belong to. They stay as the fixed
//! layout's own sections, pinned by §15 rather than stored by §14.
//!
//! The distinction is the same one `Loop::observe` already makes and states:
//! *evidence* is kept in order and is droppable; anything the harness merely
//! **decided** is rewritten in `WorkingState` rather than sitting beside its own
//! previous value. This store holds the first kind. `ADR-S9-01` records it,
//! because Set S task S-9.1 phrases the acceptance as "every block site" and the
//! four rebuilt sites are deliberately not among them.
//!
//! # Ordering is numeric, not lexical
//!
//! [`RecordId`] wraps a `u32` and renders as `r007`. The tie-break §24.6 requires
//! is `(step asc, record id)`, and a lexical id would order `r100` before `r99`
//! — a bug that appears only on runs past a hundred records, which are exactly
//! the runs this phase exists for.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::body::Body;
use crate::provenance::Provenance;

/// A stable handle to one record: what a citation, a manifest line and an
/// `expand` call all point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordId(pub u32);

impl std::fmt::Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "r{:03}", self.0)
    }
}

impl RecordId {
    /// Parse the rendered form, for `expand(record_id)` arriving from a model.
    ///
    /// Tolerant of a missing `r` and of leading zeroes, because those are the
    /// two things a model gets wrong about an id it read in a table, and a
    /// refusal there costs a step to say something the harness already knew.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        let digits = t.strip_prefix('r').or_else(|| t.strip_prefix('R')).unwrap_or(t);
        digits.parse::<u32>().ok().map(RecordId)
    }
}

/// What a record is, which drives the kind priors in selection (§15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// What a tool returned.
    ToolResult,
    /// Something the run established.
    Finding,
    /// A rejection, a validation failure, a refused call.
    Correction,
    /// The plan changed, and why.
    PlanChange,
    /// Something the memory brief retrieved.
    Memory,
    /// A choice the run made and its reason.
    Decision,
    /// Anything the harness said about itself.
    Note,
}

impl RecordKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RecordKind::ToolResult => "ToolResult",
            RecordKind::Finding => "Finding",
            RecordKind::Correction => "Correction",
            RecordKind::PlanChange => "PlanChange",
            RecordKind::Memory => "Memory",
            RecordKind::Decision => "Decision",
            RecordKind::Note => "Note",
        }
    }

    /// Every kind, for the exhaustiveness the policy's kind priors need.
    pub const ALL: &'static [RecordKind] = &[
        RecordKind::ToolResult,
        RecordKind::Finding,
        RecordKind::Correction,
        RecordKind::PlanChange,
        RecordKind::Memory,
        RecordKind::Decision,
        RecordKind::Note,
    ];
}

/// What a record is *about* — extracted by the producer, never parsed from prose.
///
/// §14 is explicit about that: a subject read out of a sentence is a subject that
/// changes when the sentence is reworded, and the near-duplicate and diversity
/// machinery joins on it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub instrument: Option<String>,
    pub timeframe: Option<String>,
    pub strategy: Option<String>,
    pub experiment: Option<String>,
    pub family: Option<String>,
}

impl Subject {
    #[must_use]
    pub fn instrument(name: impl Into<String>) -> Self {
        Self {
            instrument: Some(name.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Fraction of the *other* subject's stated fields this one matches.
    ///
    /// Asymmetric on purpose: the query is the working state, and the question
    /// selection asks is "does this record cover what the step is about", not
    /// "are these the same thing". A record about BTC-USD with no timeframe
    /// fully covers a step about BTC-USD; a record about BTC-USD 1h partly
    /// covers a step about BTC-USD 1d and must not score as a miss.
    #[must_use]
    pub fn coverage(&self, query: &Subject) -> f32 {
        let pairs = [
            (&self.instrument, &query.instrument),
            (&self.timeframe, &query.timeframe),
            (&self.strategy, &query.strategy),
            (&self.experiment, &query.experiment),
            (&self.family, &query.family),
        ];
        let mut asked = 0u32;
        let mut hit = 0u32;
        for (mine, theirs) in pairs {
            if let Some(t) = theirs {
                asked += 1;
                if mine.as_deref() == Some(t.as_str()) {
                    hit += 1;
                }
            }
        }
        if asked == 0 {
            0.0
        } else {
            hit as f32 / asked as f32
        }
    }

    /// The short label a manifest line carries.
    #[must_use]
    pub fn label(&self) -> String {
        [
            self.instrument.as_deref(),
            self.timeframe.as_deref(),
            self.strategy.as_deref(),
            self.experiment.as_deref(),
            self.family.as_deref(),
        ]
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// What the model did with a record after it was rendered (§19).
///
/// [`Utility::Indeterminate`] is not a placeholder for "we have not looked": that
/// is [`Utility::NotAttributed`]. It is the honest answer when the evidence
/// cannot separate "the model never read it" from "the model read it and
/// correctly dismissed it" — §28.2's rule, because those are a selection failure
/// and a selection success and they look identical as "rendered, not cited".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Utility {
    /// Not yet attributed. The state every record is in during the run.
    #[default]
    NotAttributed,
    /// An id of this record appeared in what the model did next.
    Used,
    /// The model asked to `expand` it — it was wanted and under-rendered.
    Wanted,
    /// Rendered and demonstrably not referenced.
    Unused,
    /// Rendered, not referenced, and the evidence cannot say whether that was a
    /// miss or a dismissal.
    Indeterminate,
}

impl Utility {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Utility::NotAttributed => "not_attributed",
            Utility::Used => "used",
            Utility::Wanted => "wanted",
            Utility::Unused => "unused",
            Utility::Indeterminate => "indeterminate",
        }
    }
}

/// One addressable observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: RecordId,
    pub step: u32,
    pub kind: RecordKind,
    /// The tool name, or `memory`, or `harness`.
    pub source: String,
    pub provenance: Provenance,
    pub subject: Subject,
    /// The evidence ids this record carries — `experiment_id`, `run_id`,
    /// `ticket_id`. Selection's strongest structural signal: the record that
    /// produced the id the next step uses is the record that step needs.
    pub ids: BTreeMap<String, String>,
    pub body: Body,
    /// Tokens of the undegraded render, fixed at insert.
    pub tokens_full: u32,
    pub utility: Utility,
    /// Set when a later record supersedes this one (§26.1).
    pub superseded_by: Option<RecordId>,
    /// How many times the model has `expand`ed it. Two is a promotion (§18):
    /// the model has now told us twice that the scoring was wrong.
    pub expansions: u32,
    /// Times an id of this record appeared in a later call, within the run.
    pub citations: u32,
}

impl Record {
    /// The key supersession joins on (§26.1).
    ///
    /// Source plus subject plus the *window* if the record names one: a second
    /// read of the same instrument, timeframe and window supersedes the first.
    /// A record with no subject at all has no key — two harness notes are not
    /// two readings of one thing, and treating them as such would silently hide
    /// the first.
    #[must_use]
    pub fn supersession_key(&self) -> Option<String> {
        if self.subject.is_empty() || self.kind != RecordKind::ToolResult {
            return None;
        }
        let window = self
            .ids
            .get("window")
            .or_else(|| self.ids.get("range"))
            .map_or("", String::as_str);
        Some(format!(
            "{}|{}|{window}",
            self.source,
            self.subject.label()
        ))
    }

    /// Whether this record is the head of its chain.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.superseded_by.is_none()
    }
}

/// What a caller hands the store; the store assigns the id and the token count.
#[derive(Debug, Clone)]
pub struct Draft {
    pub step: u32,
    pub kind: RecordKind,
    pub source: String,
    pub provenance: Provenance,
    pub subject: Subject,
    pub ids: BTreeMap<String, String>,
    pub body: Body,
}

impl Draft {
    #[must_use]
    pub fn new(step: u32, kind: RecordKind, source: impl Into<String>, body: Body) -> Self {
        Self {
            step,
            kind,
            source: source.into(),
            provenance: Provenance::ToolInternal,
            subject: Subject::default(),
            ids: BTreeMap::new(),
            body,
        }
    }

    #[must_use]
    pub fn with_provenance(mut self, p: Provenance) -> Self {
        self.provenance = p;
        self
    }

    #[must_use]
    pub fn with_subject(mut self, s: Subject) -> Self {
        self.subject = s;
        self
    }

    #[must_use]
    pub fn with_id(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.ids.insert(k.into(), v.into());
        self
    }
}

/// Every observation one run made, in insertion order.
///
/// There is no `remove`, and that is the point. The only mutation a record
/// undergoes is annotation — supersession, utility, expansion and citation
/// counts — never deletion.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunStore {
    records: Vec<Record>,
    next_id: u32,
}

impl RunStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a record and return its id.
    ///
    /// A second reading of the same thing supersedes the first (§26.1) — which
    /// is cheaper and more honest than leaving both for §24.1's redundancy
    /// penalty to suppress afterwards. Supersession *annotates*; the superseded
    /// record is still in the store, still in the manifest, and still
    /// `expand`able.
    pub fn insert(&mut self, draft: Draft) -> RecordId {
        self.next_id += 1;
        let id = RecordId(self.next_id);
        let record = Record {
            id,
            step: draft.step,
            kind: draft.kind,
            source: draft.source,
            provenance: draft.provenance,
            subject: draft.subject,
            ids: draft.ids,
            tokens_full: draft.body.tokens(),
            body: draft.body,
            utility: Utility::NotAttributed,
            superseded_by: None,
            expansions: 0,
            citations: 0,
        };
        if let Some(key) = record.supersession_key() {
            let earlier: Vec<usize> = self
                .records
                .iter()
                .enumerate()
                .filter(|(_, r)| r.is_live() && r.supersession_key().as_deref() == Some(&key))
                .map(|(i, _)| i)
                .collect();
            for i in earlier {
                self.records[i].superseded_by = Some(id);
            }
        }
        self.records.push(record);
        id
    }

    #[must_use]
    pub fn get(&self, id: RecordId) -> Option<&Record> {
        self.records.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: RecordId) -> Option<&mut Record> {
        self.records.iter_mut().find(|r| r.id == id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Every record, superseded ones included — the manifest shows those.
    pub fn all(&self) -> impl Iterator<Item = &Record> {
        self.records.iter()
    }

    /// The records selection may choose from: heads of chains only.
    ///
    /// §26.1's reason for filtering here rather than penalising later: a
    /// redundancy penalty spends effort suppressing what the store should never
    /// have offered twice, and it can only ever suppress *most* of it.
    pub fn live(&self) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(|r| r.is_live())
    }

    /// How long a supersession chain ending here is, for the manifest's
    /// collapsed line.
    #[must_use]
    pub fn chain_depth(&self, id: RecordId) -> usize {
        self.records
            .iter()
            .filter(|r| r.superseded_by == Some(id))
            .count()
    }

    /// Record that the model asked for a record back (§18).
    ///
    /// Returns the new expansion count; two is a promotion for the rest of the
    /// run.
    pub fn note_expansion(&mut self, id: RecordId) -> u32 {
        match self.get_mut(id) {
            Some(r) => {
                r.expansions += 1;
                r.utility = Utility::Wanted;
                r.expansions
            }
            None => 0,
        }
    }

    /// Record that an id of this record appeared in what the model did next.
    pub fn note_citation(&mut self, id: RecordId) {
        if let Some(r) = self.get_mut(id) {
            r.citations += 1;
            r.utility = Utility::Used;
        }
    }

    /// The ids every live record carries, for matching against a step's
    /// arguments.
    #[must_use]
    pub fn records_carrying(&self, id_value: &str) -> Vec<RecordId> {
        self.records
            .iter()
            .filter(|r| r.ids.values().any(|v| v == id_value))
            .map(|r| r.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Column;

    fn bars(step: u32, instrument: &str, timeframe: &str, window: &str) -> Draft {
        Draft::new(
            step,
            RecordKind::ToolResult,
            "get_bars",
            Body::Table {
                schema: vec![Column::new("ts", 200), Column::new("close", 200)],
                rows: vec![vec!["1".into(), "2".into()]],
                total_rows: 1,
            },
        )
        .with_subject(Subject {
            instrument: Some(instrument.into()),
            timeframe: Some(timeframe.into()),
            ..Subject::default()
        })
        .with_id("window", window)
    }

    /// The property the store exists for.
    #[test]
    fn nothing_is_ever_deleted() {
        let mut s = RunStore::new();
        let a = s.insert(bars(1, "BTC-USD", "1h", "2024-01..06"));
        let b = s.insert(bars(9, "BTC-USD", "1h", "2024-01..06"));
        assert_eq!(s.len(), 2, "a superseded record stays in the store");
        assert_eq!(s.get(a).unwrap().superseded_by, Some(b));
        assert!(s.get(a).is_some(), "and stays addressable");
        assert_eq!(s.live().count(), 1, "but selection sees only the head");
    }

    /// Supersession joins on source, subject and window — not on any two reads.
    #[test]
    fn a_different_window_is_not_a_second_reading() {
        let mut s = RunStore::new();
        s.insert(bars(1, "BTC-USD", "1h", "2024-01..06"));
        s.insert(bars(2, "BTC-USD", "1h", "2024-07..12"));
        s.insert(bars(3, "ETH-USD", "1h", "2024-01..06"));
        assert_eq!(
            s.live().count(),
            3,
            "three different readings, none superseding another"
        );
    }

    /// Two harness notes are not two readings of one thing.
    #[test]
    fn a_record_with_no_subject_has_no_supersession_key() {
        let mut s = RunStore::new();
        s.insert(Draft::new(1, RecordKind::Note, "harness", Body::text("a")));
        s.insert(Draft::new(2, RecordKind::Note, "harness", Body::text("b")));
        assert_eq!(
            s.live().count(),
            2,
            "collapsing these would silently hide the first"
        );
    }

    /// A finding is never superseded by a re-read: §26.1 is about readings.
    #[test]
    fn only_tool_results_supersede() {
        let mut s = RunStore::new();
        let subject = Subject::instrument("BTC-USD");
        s.insert(
            Draft::new(1, RecordKind::Finding, "record_finding", Body::text("a"))
                .with_subject(subject.clone()),
        );
        s.insert(
            Draft::new(4, RecordKind::Finding, "record_finding", Body::text("b"))
                .with_subject(subject),
        );
        assert_eq!(
            s.live().count(),
            2,
            "two findings about one subject may disagree, and §24.8 says both render"
        );
    }

    /// Ids order numerically, which lexical ids do not past ninety-nine.
    #[test]
    fn ids_order_numerically_past_a_hundred() {
        assert!(RecordId(99) < RecordId(100));
        assert_eq!(RecordId(7).to_string(), "r007");
        assert!(
            "r100" < "r99",
            "the lexical order this type exists to avoid"
        );
    }

    #[test]
    fn a_model_may_write_the_id_loosely() {
        for s in ["r007", "R007", "7", " r7 ", "007"] {
            assert_eq!(RecordId::parse(s), Some(RecordId(7)), "{s}");
        }
        assert_eq!(RecordId::parse("experiment_9"), None);
    }

    /// Coverage answers "does this record cover the step", not "are these equal".
    #[test]
    fn subject_coverage_is_asymmetric_on_purpose() {
        let record = Subject::instrument("BTC-USD");
        let step = Subject {
            instrument: Some("BTC-USD".into()),
            timeframe: Some("1h".into()),
            ..Subject::default()
        };
        assert!(
            (record.coverage(&step) - 0.5).abs() < f32::EPSILON,
            "a record with no timeframe half-covers a step that names one"
        );
        assert!(
            (step.coverage(&record) - 1.0).abs() < f32::EPSILON,
            "and fully covers the step that does not"
        );
        assert!((Subject::default().coverage(&step)).abs() < f32::EPSILON);
    }

    /// Expansion is counted, because twice is a promotion (§18).
    #[test]
    fn a_second_expansion_is_visible_to_the_caller() {
        let mut s = RunStore::new();
        let id = s.insert(Draft::new(1, RecordKind::Note, "harness", Body::text("a")));
        assert_eq!(s.note_expansion(id), 1);
        assert_eq!(s.note_expansion(id), 2);
        assert_eq!(s.get(id).unwrap().utility, Utility::Wanted);
        assert_eq!(
            s.note_expansion(RecordId(999)),
            0,
            "an unknown id is not an expansion"
        );
    }

    /// `tokens_full` is the undegraded size, fixed at insert.
    #[test]
    fn the_full_size_is_remembered_after_degradation() {
        let mut s = RunStore::new();
        let id = s.insert(Draft::new(
            1,
            RecordKind::ToolResult,
            "get_bars",
            Body::text("x".repeat(4000)),
        ));
        assert_eq!(s.get(id).unwrap().tokens_full, 1000);
    }
}

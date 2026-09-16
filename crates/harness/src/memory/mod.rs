//! Research memory — what the agent knows, and how it came to know it.
//!
//! AGENT-007. Set S tasks S-4.1…S-4.7, S-4.15, S-4.16.
//!
//! # The one principle this module exists to enforce
//!
//! **Memory is written by tools from verified results, never by the model's prose
//! (AGENT-007 P1, D-S5).** A model that can write its own findings can write a
//! finding that was never true, and a finding that was never true becomes an
//! authoritative premise for every later run. So every write path here is a
//! function of something a tool actually returned:
//!
//! | Entry | Written from | The model's part |
//! |---|---|---|
//! | [`Kind::Finding`] | a claim whose ids are in the run's `seen_ids` and whose numbers are in those results ([`verify`]) | proposes the claim |
//! | [`Kind::NegativeResult`] | a funnel refusal, a failed study objective or a `reject` verdict ([`negative`]) | annotates, never creates |
//! | [`Kind::InstrumentProfile`] | a scheduled template run | nothing |
//! | [`Kind::UserPreference`] | the user's own words | nothing |
//!
//! # Why this crate has no clock and no hasher
//!
//! `harness` depends on `serde`, `serde_json`, `serde_yaml`, `thiserror` and
//! `tracing` — and gate H-1 says the workspace manifest is the user's. So dates
//! arrive as ISO-8601 strings supplied by the caller and are compared with
//! [`days_between`], which is Hinnant's civil-date algorithm in fifteen lines.
//! That is not a workaround: a sans-IO loop that reads a clock is a loop whose
//! tests depend on when they run, and `tests/no_wallclock.rs` already asserts
//! the same rule one crate over.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod brief;
pub mod conflict;
pub mod negative;
pub mod retention;
pub mod turn;
pub mod verify;
pub mod window;

pub use brief::{assemble_brief, Brief, BriefItem, Detail as BriefDetail, Retrieval, Slot};
pub use conflict::{relate, Relation};
pub use negative::{derive_negative_result, FailureClass, NegativeSource, RefusedWrite};
pub use retention::{retention, Retention};
pub use turn::{SummaryFinding, SummaryNegative, TurnBudget, TurnSummary};
pub use verify::{verify_finding, FindingVerdict};

// ── Shared vocabulary (AGENT-007 §2.2) ──────────────────────────────────────

/// What kind of thing is remembered. The six §2.2 kinds plus `open_question`,
/// which AGENT-013 §9 added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Finding,
    NegativeResult,
    InstrumentProfile,
    SignalRegistry,
    PlatformDefect,
    UserPreference,
    OpenQuestion,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Finding => "finding",
            Kind::NegativeResult => "negative_result",
            Kind::InstrumentProfile => "instrument_profile",
            Kind::SignalRegistry => "signal_registry",
            Kind::PlatformDefect => "platform_defect",
            Kind::UserPreference => "user_preference",
            Kind::OpenQuestion => "open_question",
        }
    }

    /// Days a fresh entry of this kind stays valid, from the end of its data
    /// window (AGENT-007 §3.1 step 3).
    ///
    /// `None` means it never expires on a timer. A platform defect is true until
    /// someone fixes it, and a negative result is the cheapest row in the store
    /// and the likeliest to save a budget (§2.4) — expiring either would throw
    /// away the entry most worth keeping.
    #[must_use]
    pub fn validity_days(self) -> Option<i64> {
        match self {
            // The 90-day default is a guess, and AGENT-007 §10 says so. It is
            // recorded here as one number rather than sprinkled through callers
            // so the re-verification job can correct it in one place.
            Kind::Finding | Kind::SignalRegistry => Some(90),
            Kind::InstrumentProfile => Some(14),
            Kind::OpenQuestion => Some(180),
            Kind::NegativeResult | Kind::PlatformDefect | Kind::UserPreference => None,
        }
    }
}

/// How sure the platform is, not how sure the model sounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Every number in the claim was found in a cited result.
    Established,
    /// The claim cites real ids but asserts a number those results do not carry.
    Suggestive,
    /// A later comparison refuted it.
    Refuted,
}

impl Confidence {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Established => "established",
            Confidence::Suggestive => "suggestive",
            Confidence::Refuted => "refuted",
        }
    }

    /// Retrieval rank weight. `established` outranks `suggestive` (AGENT-007 §4),
    /// and a refuted entry is still retrieved — knowing an idea is dead is the
    /// point of keeping it.
    #[must_use]
    pub fn weight(self) -> f64 {
        match self {
            Confidence::Established => 1.0,
            Confidence::Suggestive => 0.5,
            Confidence::Refuted => 0.35,
        }
    }
}

/// Where the number came from (AGENT-007 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    /// A statistic a tool computed and returned.
    DirectStat,
    /// Arithmetic over direct statistics, done outside the tool.
    Derived,
    /// A model's summary of something it read. Never `established`.
    ModelSummary,
    /// The user said so.
    Human,
}

/// The period a claim is about. A finding is a claim about a window, never about
/// a market in general.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataWindow {
    /// ISO-8601 date, `YYYY-MM-DD`.
    pub start: String,
    pub end: String,
}

impl DataWindow {
    #[must_use]
    pub fn new(start: impl Into<String>, end: impl Into<String>) -> Self {
        Self {
            start: start.into(),
            end: end.into(),
        }
    }

    /// Whether two windows share any day.
    ///
    /// ISO-8601 dates compare correctly as strings, which is the whole reason the
    /// format is used here rather than anything friendlier.
    #[must_use]
    pub fn overlaps(&self, other: &DataWindow) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// Who or what produced an entry (AGENT-007 P2).
///
/// Every field is required. An entry without provenance is a rumour, and §8.3's
/// database CHECK says the same thing one layer down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub run_id: String,
    pub tool: String,
    pub tool_version: String,
    pub template_hash: String,
}

impl Provenance {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        tool: impl Into<String>,
        tool_version: impl Into<String>,
        template_hash: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            tool: tool.into(),
            tool_version: tool_version.into(),
            template_hash: template_hash.into(),
        }
    }

    /// Whether this provenance is complete enough to store.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.run_id.trim().is_empty() && !self.tool.trim().is_empty()
    }
}

/// Which way a measured quantity moved.
///
/// This is the field the contradiction check keys on, and it is set from the
/// structured result rather than parsed out of the claim's prose — §2.2.1 step 2
/// is explicit about that, because "EMA-cross does not beat momentum" and
/// "EMA-cross beats momentum" differ by one word that a parser will get wrong
/// roughly as often as it matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Positive,
    Negative,
    /// Measured and indistinguishable from zero. A null is a result.
    Null,
}

impl Direction {
    /// Whether two directions point opposite ways.
    ///
    /// `Null` opposes nothing: a null result and a positive result disagree about
    /// power, not about sign, and calling that a contradiction would flag every
    /// replication failure as a poisoned memory.
    #[must_use]
    pub fn opposes(self, other: Direction) -> bool {
        matches!(
            (self, other),
            (Direction::Positive, Direction::Negative) | (Direction::Negative, Direction::Positive)
        )
    }
}

/// The structured number behind a claim.
///
/// Without this, a contradiction check has only prose, and P5 becomes a field
/// that is always empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    /// What was measured, as a stable name: `residual_t`, `sharpe`, `hit_rate`.
    /// Two findings about different quantities never contradict.
    pub quantity: String,
    pub direction: Direction,
    pub value: f64,
    /// Standard error, when the producing tool reported one.
    pub se: Option<f64>,
    pub n: Option<u64>,
}

impl Measurement {
    #[must_use]
    pub fn new(quantity: impl Into<String>, direction: Direction, value: f64) -> Self {
        Self {
            quantity: quantity.into(),
            direction,
            value,
            se: None,
            n: None,
        }
    }

    #[must_use]
    pub fn with_se(mut self, se: f64) -> Self {
        self.se = Some(se);
        self
    }

    #[must_use]
    pub fn with_n(mut self, n: u64) -> Self {
        self.n = Some(n);
        self
    }

    /// The 95% interval, when there is a standard error to build one from.
    ///
    /// `None` when `se` is absent: a point estimate has no interval, and
    /// inventing one — treating the point as its own zero-width interval — would
    /// make every pair of unequal point estimates a contradiction.
    #[must_use]
    pub fn interval(&self) -> Option<(f64, f64)> {
        let se = self.se?;
        if !se.is_finite() || se < 0.0 {
            return None;
        }
        Some((self.value - 1.96 * se, self.value + 1.96 * se))
    }

    /// Whether the two intervals share any point. Unknown intervals do not
    /// overlap — see [`Measurement::interval`].
    #[must_use]
    pub fn intervals_overlap(&self, other: &Measurement) -> bool {
        match (self.interval(), other.interval()) {
            (Some((al, ah)), Some((bl, bh))) => al <= bh && bl <= ah,
            _ => false,
        }
    }
}

/// One thing the platform knows.
///
/// Every field AGENT-007 §2.2 names, plus the two §2.2.1/§2.5 added:
/// [`Entry::conflicts_with`] (which nothing computed in the first draft) and
/// [`Entry::corroborations`] (which turns the second independent measurement
/// into evidence rather than a duplicate row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    /// Instrument, hypothesis family, strategy or platform component.
    pub subject: String,
    /// ≤ 300 chars, enforced by [`Entry::validate`].
    pub claim: String,
    /// The ids the claim rests on, by role: `{"experiment_id": "exp_…"}`.
    pub evidence_ids: BTreeMap<String, String>,
    pub measurement: Option<Measurement>,
    pub confidence: Confidence,
    pub evidence_class: EvidenceClass,
    pub data_window: DataWindow,
    /// ISO date. `None` for kinds that never expire on a timer.
    pub valid_until: Option<String>,
    pub provenance: Provenance,
    pub created_by: String,
    pub superseded_by: Option<String>,
    pub conflicts_with: Vec<String>,
    pub stale: bool,
    pub archived: bool,
    /// How many independent runs have now measured the same thing (§2.5).
    /// Starts at 1 — the run that wrote it.
    pub corroborations: u32,
    /// The run ids that corroborated, appended in order.
    pub corroborated_by: Vec<String>,
    /// Whether an evidence chain — a sealed trial ticket or a dossier — cites
    /// this entry. Nothing cited is ever hard-deleted (§2.4).
    pub cited_by_chain: bool,
    /// Kind-specific fields: `failure_class`, `coverage`, `scope`, and the rest
    /// of §2.2's right-hand column.
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// Why an entry may not be stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    #[error("an entry without provenance is a rumour: {0}")]
    NoProvenance(String),
    #[error("claim is empty")]
    EmptyClaim,
    #[error("claim is {0} chars; the limit is 300")]
    ClaimTooLong(usize),
    #[error("subject is empty")]
    EmptySubject,
    #[error("a data window needs both ends")]
    NoWindow,
}

impl Entry {
    /// A finding, with the conservative defaults. Callers raise confidence only
    /// through [`verify::verify_finding`].
    #[must_use]
    pub fn finding(
        id: impl Into<String>,
        subject: impl Into<String>,
        claim: impl Into<String>,
        window: DataWindow,
        provenance: Provenance,
    ) -> Self {
        Self {
            id: id.into(),
            kind: Kind::Finding,
            subject: subject.into(),
            claim: claim.into(),
            evidence_ids: BTreeMap::new(),
            measurement: None,
            confidence: Confidence::Suggestive,
            evidence_class: EvidenceClass::Derived,
            data_window: window,
            valid_until: None,
            provenance,
            created_by: String::new(),
            superseded_by: None,
            conflicts_with: Vec::new(),
            stale: false,
            archived: false,
            corroborations: 1,
            corroborated_by: Vec::new(),
            cited_by_chain: false,
            extra: serde_json::Value::Null,
        }
    }

    #[must_use]
    pub fn with_measurement(mut self, m: Measurement) -> Self {
        self.measurement = Some(m);
        self
    }

    #[must_use]
    pub fn with_evidence(mut self, role: impl Into<String>, id: impl Into<String>) -> Self {
        self.evidence_ids.insert(role.into(), id.into());
        self
    }

    /// Set `valid_until` from the window's end and the kind's policy
    /// (AGENT-007 §3.1 step 3). Idempotent; a kind with no expiry leaves it
    /// `None`.
    #[must_use]
    pub fn expiring(mut self) -> Self {
        self.valid_until = self
            .kind
            .validity_days()
            .and_then(|d| add_days(&self.data_window.end, d));
        self
    }

    /// Whether this entry may be stored at all.
    ///
    /// The same four rules migration 0066 states as CHECK constraints. Duplicated
    /// deliberately: the database is the last line, and a caller that gets a
    /// typed error before the round-trip gets a better message.
    pub fn validate(&self) -> Result<(), EntryError> {
        if !self.provenance.is_complete() {
            return Err(EntryError::NoProvenance(self.id.clone()));
        }
        if self.claim.trim().is_empty() {
            return Err(EntryError::EmptyClaim);
        }
        if self.claim.chars().count() > 300 {
            return Err(EntryError::ClaimTooLong(self.claim.chars().count()));
        }
        if self.subject.trim().is_empty() {
            return Err(EntryError::EmptySubject);
        }
        if self.data_window.start.is_empty() || self.data_window.end.is_empty() {
            return Err(EntryError::NoWindow);
        }
        Ok(())
    }

    /// The one-line brief rendering (AGENT-007 §4).
    ///
    /// `[f_31a, established, n=812, 2026-04-01→2026-06-30] BTC-USD 1h: …`
    #[must_use]
    pub fn line(&self) -> String {
        let n = self
            .measurement
            .as_ref()
            .and_then(|m| m.n)
            .map_or_else(String::new, |n| format!(", n={n}"));
        let corrob = if self.corroborations > 1 {
            format!(", corroborated×{}", self.corroborations)
        } else {
            String::new()
        };
        let flags = match (self.stale, self.conflicts_with.is_empty()) {
            (true, false) => ", STALE, CONFLICTED",
            (true, true) => ", STALE",
            (false, false) => ", CONFLICTED",
            (false, true) => "",
        };
        format!(
            "[{}, {}{n}{corrob}, {}→{}{flags}] {}: {}",
            self.id,
            self.confidence.as_str(),
            self.data_window.start,
            self.data_window.end,
            self.subject,
            self.claim
        )
    }
}

// ── Dates, without a clock ──────────────────────────────────────────────────

/// Parse `YYYY-MM-DD` into days since 1970-01-01.
///
/// Hinnant's `days_from_civil`. Exact for every date this platform will ever
/// hold, and — unlike anything that reads a clock — the same answer on every
/// machine on every day.
#[must_use]
pub fn days_from_civil(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = iso.get(0..4)?.parse().ok()?;
    let m: i64 = iso.get(5..7)?.parse().ok()?;
    let d: i64 = iso.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The inverse, so `valid_until` can be computed without a date library.
#[must_use]
pub fn civil_from_days(mut z: i64) -> String {
    z += 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `to - from` in days. `None` if either does not parse.
#[must_use]
pub fn days_between(from: &str, to: &str) -> Option<i64> {
    Some(days_from_civil(to)? - days_from_civil(from)?)
}

/// `iso + days`, as an ISO date.
#[must_use]
pub fn add_days(iso: &str, days: i64) -> Option<String> {
    Some(civil_from_days(days_from_civil(iso)? + days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip_across_a_leap_boundary() {
        for iso in [
            "1970-01-01",
            "2000-02-29",
            "2026-09-15",
            "2024-12-31",
            "1999-03-01",
        ] {
            let d = days_from_civil(iso).expect(iso);
            assert_eq!(civil_from_days(d), iso, "{iso}");
        }
    }

    #[test]
    fn day_arithmetic_crosses_a_month_and_a_year() {
        assert_eq!(add_days("2026-06-30", 90).as_deref(), Some("2026-09-28"));
        assert_eq!(add_days("2026-12-20", 30).as_deref(), Some("2027-01-19"));
        assert_eq!(days_between("2026-01-01", "2026-12-31"), Some(364));
    }

    #[test]
    fn a_malformed_date_is_none_rather_than_a_wrong_number() {
        assert!(days_from_civil("not-a-date").is_none());
        assert!(days_from_civil("2026-13-01").is_none());
        assert!(days_from_civil("2026-9-15").is_none());
    }

    #[test]
    fn windows_overlap_on_a_shared_day_and_not_otherwise() {
        let a = DataWindow::new("2026-01-01", "2026-06-30");
        assert!(a.overlaps(&DataWindow::new("2026-06-30", "2026-12-31")));
        assert!(!a.overlaps(&DataWindow::new("2026-07-01", "2026-12-31")));
    }

    fn prov() -> Provenance {
        Provenance::new("run_1", "run_backtest", "v3", "tpl_abc")
    }

    #[test]
    fn an_entry_without_provenance_cannot_be_stored() {
        let mut e = Entry::finding(
            "f_1",
            "BTC-USD",
            "no edge",
            DataWindow::new("2026-01-01", "2026-06-30"),
            Provenance::new("", "", "", ""),
        );
        assert!(matches!(e.validate(), Err(EntryError::NoProvenance(_))));
        e.provenance = prov();
        assert!(e.validate().is_ok());
    }

    #[test]
    fn a_claim_over_three_hundred_chars_is_refused() {
        let mut e = Entry::finding(
            "f_1",
            "BTC-USD",
            "x",
            DataWindow::new("2026-01-01", "2026-06-30"),
            prov(),
        );
        e.claim = "a".repeat(301);
        assert!(matches!(e.validate(), Err(EntryError::ClaimTooLong(301))));
    }

    #[test]
    fn validity_comes_from_the_window_end_and_the_kind() {
        let e = Entry::finding(
            "f_1",
            "BTC-USD",
            "x",
            DataWindow::new("2026-01-01", "2026-06-30"),
            prov(),
        )
        .expiring();
        assert_eq!(e.valid_until.as_deref(), Some("2026-09-28"));

        let mut neg = e.clone();
        neg.kind = Kind::NegativeResult;
        let neg = neg.expiring();
        assert!(
            neg.valid_until.is_none(),
            "a negative result is the cheapest row in the store; it does not expire"
        );
    }

    #[test]
    fn a_point_estimate_has_no_interval_and_does_not_overlap_anything() {
        let a = Measurement::new("sharpe", Direction::Positive, 0.4);
        let b = Measurement::new("sharpe", Direction::Positive, 0.4);
        assert!(a.interval().is_none());
        assert!(!a.intervals_overlap(&b));
    }

    #[test]
    fn intervals_overlap_when_the_errors_are_wide_enough() {
        let a = Measurement::new("sharpe", Direction::Positive, 0.4).with_se(0.2);
        let b = Measurement::new("sharpe", Direction::Negative, -0.1).with_se(0.2);
        assert!(a.intervals_overlap(&b));
        let c = Measurement::new("sharpe", Direction::Negative, -0.9).with_se(0.01);
        assert!(!a.intervals_overlap(&c));
    }

    #[test]
    fn a_null_direction_opposes_nothing() {
        assert!(Direction::Positive.opposes(Direction::Negative));
        assert!(!Direction::Null.opposes(Direction::Positive));
        assert!(!Direction::Positive.opposes(Direction::Positive));
    }

    #[test]
    fn the_brief_line_never_loses_the_id() {
        let mut e = Entry::finding(
            "f_31a",
            "BTC-USD 1h",
            "EMA-cross edge is spanned by momentum",
            DataWindow::new("2026-04-01", "2026-06-30"),
            prov(),
        )
        .with_measurement(Measurement::new("residual_t", Direction::Null, 0.4).with_n(812));
        e.confidence = Confidence::Established;
        let line = e.line();
        assert!(line.starts_with("[f_31a, established, n=812, 2026-04-01→2026-06-30]"));
        e.stale = true;
        e.conflicts_with.push("f_77".into());
        assert!(e.line().contains("STALE, CONFLICTED"));
        assert!(e.line().contains("f_31a"));
    }
}

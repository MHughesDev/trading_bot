//! What happened when a skill ran, and what that accumulates into.
//!
//! Every execution is recorded, including the failures — especially the
//! failures. A library that keeps only its successes is a library whose
//! statistics say every skill is perfect, and the point of recording at all is
//! that the agent can tell a skill that works from one that does not without
//! anybody training anything.
//!
//! [`SkillStats`] is deliberately arithmetic rather than a model. The external
//! review's argument is that alpha feedback is ~24 noisy labels a year and
//! process feedback is thousands of unambiguous ones; counting the unambiguous
//! ones is a small claim this platform can support, and fitting something to
//! them is not yet.

use serde::{Deserialize, Serialize};

use crate::contract::Side;
use crate::SkillHash;

/// How one run ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// Ran, returned an object, and the object satisfied the contract.
    Ok,
    /// The caller's payload did not satisfy the declared inputs. **Not counted
    /// against the skill** — see [`SkillStats`].
    BadInput { detail: String },
    /// The skill raised.
    Raised { exception: String },
    /// The skill returned something, and it did not satisfy the contract.
    BrokeContract { detail: String },
    /// Exceeded the wall-clock budget.
    TimedOut { limit_secs: u64 },
    /// Exceeded the memory budget, or the runner killed it.
    Killed { reason: String },
    /// The runner itself failed. Also not counted against the skill: a runner
    /// that cannot start is not evidence about the code it did not run.
    RunnerFailed { detail: String },
}

impl Outcome {
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Whether this outcome is evidence about the *skill*.
    ///
    /// A bad payload and a broken runner are facts about the caller and the
    /// platform. Counting them against the skill would make a good skill look
    /// unreliable, which is exactly the number the agent would then act on.
    #[must_use]
    pub const fn is_about_the_skill(&self) -> bool {
        !matches!(self, Self::BadInput { .. } | Self::RunnerFailed { .. })
    }

    /// A short stable code, for grouping.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::BadInput { .. } => "bad_input",
            Self::Raised { .. } => "raised",
            Self::BrokeContract { .. } => "broke_contract",
            Self::TimedOut { .. } => "timed_out",
            Self::Killed { .. } => "killed",
            Self::RunnerFailed { .. } => "runner_failed",
        }
    }
}

impl From<crate::contract::Violation> for Outcome {
    fn from(v: crate::contract::Violation) -> Self {
        match v.blames() {
            Side::Input => Self::BadInput { detail: v.to_string() },
            Side::Output => Self::BrokeContract { detail: v.to_string() },
        }
    }
}

/// One execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub skill: SkillHash,
    pub outcome: Outcome,
    pub duration_ms: u64,
    pub peak_memory_bytes: Option<u64>,
    /// Anything the skill printed. Kept because `print` is how a Python author
    /// debugs, and a runner that swallows stdout makes a skill undebuggable
    /// from outside.
    pub stdout_note: String,
    pub ran_at: chrono::DateTime<chrono::Utc>,
}

impl RunRecord {
    #[must_use]
    pub fn new(skill: SkillHash, outcome: Outcome, duration_ms: u64) -> Self {
        Self {
            skill,
            outcome,
            duration_ms,
            peak_memory_bytes: None,
            stdout_note: String::new(),
            ran_at: chrono::Utc::now(),
        }
    }
}

/// What a skill's history adds up to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillStats {
    pub runs: u64,
    pub ok: u64,
    /// Failures that are the skill's fault.
    pub failures: u64,
    /// Rejections that are the caller's fault. Reported, never held against the
    /// skill.
    pub bad_input: u64,
    /// Runner failures. Also not the skill's fault.
    pub runner_failed: u64,
    pub total_duration_ms: u64,
}

impl SkillStats {
    /// Fold one run in.
    pub fn record(&mut self, outcome: &Outcome, duration_ms: u64) {
        self.runs += 1;
        self.total_duration_ms += duration_ms;
        match outcome {
            Outcome::Ok => self.ok += 1,
            Outcome::BadInput { .. } => self.bad_input += 1,
            Outcome::RunnerFailed { .. } => self.runner_failed += 1,
            _ => self.failures += 1,
        }
    }

    /// Successes over runs that were actually about the skill.
    ///
    /// `None` until the skill has been run in anger. A reliability of 0/0 is not
    /// zero and not one, and rendering either would be the same mistake the
    /// five-state self-monitor exists to avoid: an absence of evidence shown as
    /// a number people read as evidence.
    #[must_use]
    pub fn reliability(&self) -> Option<f64> {
        let attributable = self.ok + self.failures;
        (attributable > 0).then(|| self.ok as f64 / attributable as f64)
    }

    /// Mean duration over every run, or `None` when there have been none.
    #[must_use]
    pub fn mean_duration_ms(&self) -> Option<f64> {
        (self.runs > 0).then(|| self.total_duration_ms as f64 / self.runs as f64)
    }

    /// A phrase for a reader choosing between skills.
    ///
    /// Words rather than a bare ratio, because "1.00" from a single run and
    /// "0.97" from three hundred are very different recommendations and the
    /// numbers do not say so.
    #[must_use]
    pub fn confidence(&self) -> &'static str {
        let attributable = self.ok + self.failures;
        match (attributable, self.reliability()) {
            (0, _) => "never run",
            (1..=4, _) => "barely tried",
            (_, Some(r)) if r >= 0.95 => "reliable",
            (_, Some(r)) if r >= 0.7 => "mostly works",
            (_, Some(_)) => "unreliable",
            (_, None) => "never run",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Violation;

    fn hash() -> SkillHash {
        SkillHash::of("def run(p):\n    return p\n")
    }

    // ── attribution ──────────────────────────────────────────────────────────

    /// The property the whole statistic depends on: a good skill handed a bad
    /// payload must not look unreliable.
    #[test]
    fn a_bad_payload_is_not_held_against_the_skill() {
        let mut s = SkillStats::default();
        for _ in 0..50 {
            s.record(&Outcome::BadInput { detail: "missing x".into() }, 1);
        }
        s.record(&Outcome::Ok, 10);

        assert_eq!(s.runs, 51);
        assert_eq!(s.bad_input, 50);
        assert_eq!(s.failures, 0);
        assert_eq!(s.reliability(), Some(1.0), "the skill worked every time it was asked properly");
    }

    #[test]
    fn a_runner_failure_is_not_evidence_about_the_code_it_did_not_run() {
        let mut s = SkillStats::default();
        s.record(&Outcome::RunnerFailed { detail: "no interpreter".into() }, 0);
        assert_eq!(s.runner_failed, 1);
        assert_eq!(s.failures, 0);
        assert_eq!(s.reliability(), None, "nothing attributable has happened yet");
    }

    #[test]
    fn every_other_failure_counts_against_the_skill() {
        let mut s = SkillStats::default();
        s.record(&Outcome::Raised { exception: "KeyError".into() }, 5);
        s.record(&Outcome::BrokeContract { detail: "no `sharpe`".into() }, 5);
        s.record(&Outcome::TimedOut { limit_secs: 30 }, 30_000);
        s.record(&Outcome::Killed { reason: "memory".into() }, 900);
        assert_eq!(s.failures, 4);
        assert_eq!(s.reliability(), Some(0.0));
    }

    #[test]
    fn a_violation_becomes_the_outcome_that_blames_the_right_side() {
        let input: Outcome = Violation::MissingInputs { missing: vec!["a".into()] }.into();
        assert!(matches!(input, Outcome::BadInput { .. }));
        assert!(!input.is_about_the_skill());

        let output: Outcome = Violation::MissingOutputs { missing: vec!["b".into()] }.into();
        assert!(matches!(output, Outcome::BrokeContract { .. }));
        assert!(output.is_about_the_skill());
    }

    // ── reporting absence ────────────────────────────────────────────────────

    /// The same mistake the five-state self-monitor exists to avoid: an absence
    /// of evidence rendered as a number people read as evidence.
    #[test]
    fn a_skill_that_has_never_run_has_no_reliability_rather_than_zero() {
        let s = SkillStats::default();
        assert_eq!(s.reliability(), None);
        assert_eq!(s.mean_duration_ms(), None);
        assert_eq!(s.confidence(), "never run");
    }

    /// "1.00" from one run and "0.97" from three hundred are very different
    /// recommendations, and the numbers do not say so.
    #[test]
    fn confidence_distinguishes_a_lucky_first_run_from_a_track_record() {
        let mut new = SkillStats::default();
        new.record(&Outcome::Ok, 1);
        assert_eq!(new.reliability(), Some(1.0));
        assert_eq!(new.confidence(), "barely tried");

        let mut proven = SkillStats::default();
        for _ in 0..300 {
            proven.record(&Outcome::Ok, 1);
        }
        assert_eq!(proven.confidence(), "reliable");
    }

    #[test]
    fn confidence_reads_the_middle_and_the_bottom() {
        let mut s = SkillStats::default();
        for _ in 0..80 {
            s.record(&Outcome::Ok, 1);
        }
        for _ in 0..20 {
            s.record(&Outcome::Raised { exception: "x".into() }, 1);
        }
        assert_eq!(s.confidence(), "mostly works");

        let mut bad = SkillStats::default();
        for _ in 0..10 {
            bad.record(&Outcome::Ok, 1);
        }
        for _ in 0..90 {
            bad.record(&Outcome::Raised { exception: "x".into() }, 1);
        }
        assert_eq!(bad.confidence(), "unreliable");
    }

    #[test]
    fn mean_duration_covers_every_run_including_the_failures() {
        let mut s = SkillStats::default();
        s.record(&Outcome::Ok, 100);
        s.record(&Outcome::TimedOut { limit_secs: 1 }, 1_000);
        assert_eq!(s.mean_duration_ms(), Some(550.0));
    }

    #[test]
    fn outcome_codes_are_stable_and_distinct() {
        let all = [
            Outcome::Ok,
            Outcome::BadInput { detail: String::new() },
            Outcome::Raised { exception: String::new() },
            Outcome::BrokeContract { detail: String::new() },
            Outcome::TimedOut { limit_secs: 1 },
            Outcome::Killed { reason: String::new() },
            Outcome::RunnerFailed { detail: String::new() },
        ];
        let codes: std::collections::BTreeSet<_> = all.iter().map(Outcome::code).collect();
        assert_eq!(codes.len(), all.len(), "codes must be distinct");
    }

    #[test]
    fn a_run_record_round_trips_through_json() {
        let r = RunRecord::new(hash(), Outcome::Ok, 42);
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RunRecord>(&j).unwrap(), r);
    }
}

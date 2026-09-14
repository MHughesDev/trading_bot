//! The auditor suite (AGENT-004 §4).
//!
//! Pairs of fixtures — one violating, one a clean twin — run directly against the
//! platform's own guards. No agent, no model, no credential, no network. It runs in
//! CI on every PR that touches the guards, and its target is two numbers:
//!
//! - **100% of violations caught.** A guard that misses is worse than no guard,
//!   because the pipeline behind it is built on the assumption that it does not.
//! - **0 false rejections on the clean twins.** This is the half that is usually
//!   missing. A leakage check that rejects honest work teaches the agent to route
//!   around it, and a check everyone routes around catches nothing at all.
//!
//! Every fixture is a pair for exactly that reason: a rule that fires on the
//! violating input proves nothing until the same rule stays silent on the twin that
//! differs only in the violation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which guard a fixture exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// `backtest::gates::integrity_scan` — Gate 0.
    Gate0,
    /// `report_validator::validate` — the `final_report` contract.
    FinalReport,
    /// `leakage_lint::lint_code` — static leakage lint over agent-written code.
    LeakageLint,
    /// `leakage_lint::lint_skill` — tuned constants in a skill body.
    SkillLint,
    /// `data_admission::admit_experiment` — DA-07 grade gating.
    DataGrade,
    /// `data_admission::check_revision_pinning` — revised-data use.
    RevisionPinning,
    /// A prediction series overlapping its own training window.
    PredictionOverlap,
    /// Oversized tool results are marked, not silently cut.
    Truncation,
}

/// One (violating, clean) pair.
#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub check: Check,
    /// Why this pair exists — what goes wrong in production if the guard misses.
    /// Required, and read by a human, not by code.
    pub why: String,
    /// The input that must be caught.
    pub violating: Value,
    /// The same input with the violation removed. Must be accepted.
    pub clean: Value,
    /// Codes the violating input must raise. Empty means "any finding will do",
    /// which is deliberately allowed: pinning a code makes the fixture brittle when
    /// the guard is refactored, and for some fixtures "it was refused" is the whole
    /// requirement.
    #[serde(default)]
    pub expect: Vec<String>,
}

/// What one fixture did.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub id: String,
    pub check: Check,
    /// The violating input raised at least one finding (and the expected codes, if
    /// the fixture named any).
    pub caught: bool,
    /// The clean twin raised nothing.
    pub clean_accepted: bool,
    pub violating_codes: Vec<String>,
    pub clean_codes: Vec<String>,
    /// Populated when the fixture named codes the violating input did not raise.
    pub missing_codes: Vec<String>,
}

impl Outcome {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.caught && self.clean_accepted
    }
}

/// The suite's two headline numbers (AGENT-004 §4).
#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub fixtures: usize,
    pub caught: usize,
    pub false_rejections: usize,
    pub outcomes: Vec<Outcome>,
}

impl Scorecard {
    #[must_use]
    pub fn catch_rate(&self) -> f64 {
        if self.fixtures == 0 {
            return 0.0;
        }
        self.caught as f64 / self.fixtures as f64
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.fixtures > 0 && self.caught == self.fixtures && self.false_rejections == 0
    }
}

/// Runs one guard over one input and returns the codes it raised.
///
/// A malformed input returns a `fixture.malformed_input` code rather than panicking.
/// A fixture that does not parse is a broken fixture, and a broken fixture reporting
/// "caught" would be the worst possible outcome: a suite that passes because its own
/// inputs are unreadable.
#[must_use]
pub fn run_check(check: Check, input: &Value) -> Vec<String> {
    match check {
        Check::Gate0 => gate0_codes(input),
        Check::FinalReport => final_report_codes(input),
        Check::LeakageLint => match input.get("code").and_then(Value::as_str) {
            Some(src) => crate::leakage_lint::lint_code(src)
                .into_iter()
                .filter(|f| crate::leakage_lint::is_rejection(f.code))
                .map(|f| f.code.to_string())
                .collect(),
            None => vec!["fixture.malformed_input".into()],
        },
        Check::SkillLint => match input.get("code").and_then(Value::as_str) {
            Some(src) => crate::leakage_lint::lint_skill(src)
                .into_iter()
                .map(|f| f.code.to_string())
                .collect(),
            None => vec!["fixture.malformed_input".into()],
        },
        Check::DataGrade => data_grade_codes(input),
        Check::RevisionPinning => {
            let confirmatory = input
                .get("confirmatory")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let manifest = input.get("manifest").unwrap_or(input);
            match crate::data_admission::check_revision_pinning(manifest, confirmatory) {
                Ok(()) => vec![],
                Err(r) => vec![r.code.to_string()],
            }
        }
        Check::PredictionOverlap => prediction_overlap_codes(input),
        Check::Truncation => truncation_codes(input),
    }
}

fn gate0_codes(input: &Value) -> Vec<String> {
    use backtest::gates::{integrity_scan, IntegrityInputs, SignalStamp};

    let signals: Vec<SignalStamp> = input
        .get("signals")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(SignalStamp {
                        acted_at_ns: s.get("acted_at_ns")?.as_i64()?,
                        bar_close_ns: s.get("bar_close_ns")?.as_i64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let inputs = IntegrityInputs {
        signals: &signals,
        gross_return: input
            .get("gross_return")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        cost_floor: input
            .get("cost_floor")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        label_horizon_bars: input.get("label_horizon_bars").and_then(Value::as_i64),
        feature_window_end_bar: input.get("feature_window_end_bar").and_then(Value::as_i64),
        purge_bars: input.get("purge_bars").and_then(Value::as_i64),
    };
    integrity_scan(&inputs)
        .into_iter()
        .map(|f| f.code)
        .collect()
}

fn final_report_codes(input: &Value) -> Vec<String> {
    match serde_json::from_value::<crate::report_validator::FinalReport>(input.clone()) {
        Ok(report) => crate::report_validator::validate(&report)
            .into_iter()
            .map(|e| e.rule)
            .collect(),
        Err(_) => vec!["fixture.malformed_input".into()],
    }
}

fn data_grade_codes(input: &Value) -> Vec<String> {
    let assessed = input.get("grade").and_then(Value::as_str).unwrap_or("");
    let waiver = input.get("waiver").and_then(|w| {
        Some(crate::data_admission::Waiver {
            granted_for: w.get("granted_for")?.as_str()?.to_string(),
            approved_by: w
                .get("approved_by")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            reason: w
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    });
    match crate::data_admission::admit_experiment(assessed, waiver.as_ref()) {
        Ok(()) => vec![],
        Err(r) => vec![r.code.to_string()],
    }
}

/// A prediction series must not overlap the window the model was trained on.
///
/// The check is `DataSlice::overlaps`, which the holdout vault already relies on.
/// Running it here as a fixture is not redundant: the vault uses it on one code
/// path, and this asserts that the *rule* holds for prediction series too, which is
/// where the mistake actually gets made — a model retrained on more data and then
/// scored over a window that now includes some of its own training rows.
fn prediction_overlap_codes(input: &Value) -> Vec<String> {
    use backtest::run::config::{DataSlice, EvalResolution};
    use chrono::{DateTime, Utc};

    let parse = |key: &str| -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let v = input.get(key)?;
        let start = v.get("start")?.as_str()?.parse::<DateTime<Utc>>().ok()?;
        let end = v.get("end")?.as_str()?.parse::<DateTime<Utc>>().ok()?;
        Some((start, end))
    };
    let (Some((ts, te)), Some((ps, pe))) = (parse("train"), parse("predict")) else {
        return vec!["fixture.malformed_input".into()];
    };
    let train = DataSlice::new("u", ts, te, EvalResolution::Min1);
    let predict = DataSlice::new("u", ps, pe, EvalResolution::Min1);
    if train.overlaps(&predict) {
        vec!["lookahead.prediction_train_overlap".into()]
    } else {
        vec![]
    }
}

/// An oversized tool result must be *marked* as truncated.
///
/// The failure this guards against is not a lost byte, it is a confident answer
/// computed over a prefix. A model that is told the result was cut asks for a
/// summary; a model that is not, reasons over half a table and reports a number as
/// though it had seen all of it.
///
/// This check reads the other way round from the rest of the suite, and the pair
/// still works. The "violating" input is an oversized payload, and what must be
/// detected is that the transcript *says so*: it raises `transcript.truncated`. The
/// clean twin is a small payload, which must pass through untouched and raise
/// nothing. A silent cut raises `transcript.silent_truncation`, which does not match
/// the fixture's expected code and therefore fails the suite — which is the outcome
/// we want if the marker is ever dropped.
fn truncation_codes(input: &Value) -> Vec<String> {
    // A payload of a stated size, so a fixture does not have to carry 40 KB of
    // filler in a file a human is meant to read.
    let payload = match input.get("payload_bytes").and_then(Value::as_u64) {
        Some(n) => serde_json::json!({ "filler": "x".repeat(n as usize) }),
        None => input.get("payload").cloned().unwrap_or(input.clone()),
    };
    let out = crate::agent::driver::truncate_result(&payload);
    let serialized = serde_json::to_string(&payload).unwrap_or_default();

    if out.len() >= serialized.len() {
        // Nothing was cut. Correct for a small payload; for an oversized one the
        // fixture's expected code will be missing and the suite will say so.
        return vec![];
    }
    if out.contains("truncated") {
        vec!["transcript.truncated".into()]
    } else {
        vec!["transcript.silent_truncation".into()]
    }
}

/// Runs one fixture pair.
#[must_use]
pub fn run_fixture(f: &Fixture) -> Outcome {
    let violating_codes = run_check(f.check, &f.violating);
    let clean_codes = run_check(f.check, &f.clean);

    let missing_codes: Vec<String> = f
        .expect
        .iter()
        .filter(|want| !violating_codes.iter().any(|got| got == *want))
        .cloned()
        .collect();

    Outcome {
        id: f.id.clone(),
        check: f.check,
        caught: !violating_codes.is_empty() && missing_codes.is_empty(),
        clean_accepted: clean_codes.is_empty(),
        violating_codes,
        clean_codes,
        missing_codes,
    }
}

/// Runs a whole set of fixtures.
#[must_use]
pub fn run_suite(fixtures: &[Fixture]) -> Scorecard {
    let outcomes: Vec<Outcome> = fixtures.iter().map(run_fixture).collect();
    Scorecard {
        fixtures: outcomes.len(),
        caught: outcomes.iter().filter(|o| o.caught).count(),
        false_rejections: outcomes.iter().filter(|o| !o.clean_accepted).count(),
        outcomes,
    }
}

/// Loads every `*.json` fixture in a directory.
///
/// A file that does not parse is an error rather than a skip. A suite that silently
/// ignored unreadable fixtures would report a perfect score for doing nothing, and
/// that is precisely the failure mode this whole module exists to prevent.
pub fn load_dir(dir: &std::path::Path) -> anyhow::Result<Vec<Fixture>> {
    let mut fixtures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let parsed: Vec<Fixture> = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        fixtures.extend(parsed);
    }
    Ok(fixtures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_fixture_whose_violation_is_missed_fails_rather_than_passes() {
        let f = Fixture {
            id: "impossible".into(),
            check: Check::Gate0,
            why: "a guard that misses is worse than no guard".into(),
            // Clean inputs on both sides: nothing to catch.
            violating: json!({"gross_return": 1.0, "cost_floor": 0.0}),
            clean: json!({"gross_return": 1.0, "cost_floor": 0.0}),
            expect: vec![],
        };
        let o = run_fixture(&f);
        assert!(!o.caught);
        assert!(o.clean_accepted);
        assert!(!o.passed());
    }

    #[test]
    fn a_fixture_whose_clean_twin_is_rejected_fails() {
        let f = Fixture {
            id: "over_eager".into(),
            check: Check::Gate0,
            why: "a check that rejects honest work gets routed around".into(),
            violating: json!({"gross_return": 0.0, "cost_floor": 0.1}),
            // Also below the floor: the "clean" twin is not clean.
            clean: json!({"gross_return": 0.0, "cost_floor": 0.1}),
            expect: vec!["cost.below_floor".into()],
        };
        let o = run_fixture(&f);
        assert!(o.caught);
        assert!(!o.clean_accepted);
        assert!(!o.passed());
    }

    #[test]
    fn naming_a_code_the_guard_does_not_raise_is_a_failure() {
        let f = Fixture {
            id: "wrong_code".into(),
            check: Check::Gate0,
            why: "the fixture must pin the right rule".into(),
            violating: json!({"gross_return": 0.0, "cost_floor": 0.1}),
            clean: json!({"gross_return": 1.0, "cost_floor": 0.1}),
            expect: vec!["lookahead.higher_tf_open".into()],
        };
        let o = run_fixture(&f);
        assert!(!o.caught, "the expected code was not raised");
        assert_eq!(
            o.missing_codes,
            vec!["lookahead.higher_tf_open".to_string()]
        );
    }

    #[test]
    fn a_malformed_fixture_input_is_a_visible_code_not_a_panic() {
        assert_eq!(
            run_check(Check::FinalReport, &json!({"nonsense": true})),
            vec!["fixture.malformed_input".to_string()]
        );
        assert_eq!(
            run_check(Check::LeakageLint, &json!({})),
            vec!["fixture.malformed_input".to_string()]
        );
    }

    #[test]
    fn an_empty_suite_does_not_score_as_perfect() {
        let card = run_suite(&[]);
        assert!(!card.passed(), "zero fixtures is not a pass");
        assert_eq!(card.catch_rate(), 0.0);
    }
}

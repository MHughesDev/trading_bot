//! The alpha-chain audit: five stages, each read from gate verdicts already on record
//! (Set Q Q-1.7; BS-007 11 §8 `alpha_chain_audit`).
//!
//! The chain asks, in order, whether an apparent edge is real *and* collectable:
//!
//! 1. **PIT information** - was the data it used knowable at the time? (Gate 2, the leakage suite)
//! 2. **Stable signal** - does it survive resampling and the family it was picked from?
//!    (Gates 7 PBO, 8 deflated Sharpe, 13 stationary bootstrap, 14 Romano-Wolf)
//! 3. **Feasible positions** - are the positions it implies spread widely enough to hold?
//!    (Gate 12 perturbation and concentration)
//! 4. **Executable orders** - do paper orders behave like the backtest's? (Gate 15 paper/shadow)
//! 5. **Net alpha after costs, with capacity** - what is left after costs, at size?
//!    (Gates 3 cost sensitivity and 4 capacity)
//!
//! The mapping is a judgement (ADR-Q-08): the spec names the stages and the stack's gates but
//! not which gate answers which. It is a table in this file, with a test that every gate it
//! names exists in the sixteen.
//!
//! # The rule that makes it an audit
//!
//! A stage is `Pass` **only when every gate it reads has a row and every row passed.** A missing
//! row, or a row that failed without a statistic (the stack records an inconclusive gate as a
//! structural failure: it could not run), is `Unknown`, never `Pass`. A row that failed with a
//! statistic is `Fail` and takes precedence over an unknown sibling, because a measured failure
//! is information and a hole is not. Each stage names the rows it read, so a verdict can be
//! traced to the gate row behind it; there is no stage whose verdict is a restatement.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One recorded gate verdict, as the audit reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateRow {
    pub gate_no: i32,
    pub gate_name: String,
    pub profile_id: String,
    pub passed: bool,
    pub statistic: Option<f64>,
    pub detail: String,
    pub decided_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageVerdict {
    Pass,
    Fail,
    Unknown,
}

/// The five stages, with the gates each reads.
pub const STAGES: [(&str, &[i32]); 5] = [
    ("pit_information", &[2]),
    ("stable_signal", &[7, 8, 13, 14]),
    ("feasible_positions", &[12]),
    ("executable_orders", &[15]),
    ("net_alpha_after_costs", &[3, 4]),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RowRead {
    pub gate_no: i32,
    /// `present`, or `absent` when no row exists for this gate on the experiment.
    pub state: &'static str,
    pub passed: Option<bool>,
    pub profile_id: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub stage: &'static str,
    pub verdict: StageVerdict,
    /// Which rows were read, absent ones included: the stage's evidence, or the lack of it.
    pub rows_read: Vec<RowRead>,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlphaChain {
    pub stages: Vec<Stage>,
    /// The first stage that is not `Pass`: where the chain breaks, or where it has no evidence.
    pub first_unresolved: Option<&'static str>,
    /// Every stage passed. Never true while any stage is `Unknown`.
    pub complete: bool,
}

/// The latest row per gate: the newest verdict is the standing one.
fn latest(rows: &[GateRow], gate: i32) -> Option<&GateRow> {
    rows.iter()
        .filter(|r| r.gate_no == gate)
        .max_by_key(|r| r.decided_at)
}

/// Audit one experiment's rows.
#[must_use]
pub fn audit(rows: &[GateRow]) -> AlphaChain {
    let mut stages = Vec::with_capacity(STAGES.len());
    for (name, gates) in STAGES {
        let mut read = Vec::new();
        let (mut failed, mut unknown, mut passed) = (Vec::new(), Vec::new(), 0usize);
        for &g in gates {
            match latest(rows, g) {
                None => {
                    read.push(RowRead {
                        gate_no: g,
                        state: "absent",
                        passed: None,
                        profile_id: None,
                        decided_at: None,
                    });
                    unknown.push(format!("gate {g} has no row"));
                }
                Some(r) => {
                    read.push(RowRead {
                        gate_no: g,
                        state: "present",
                        passed: Some(r.passed),
                        profile_id: Some(r.profile_id.clone()),
                        decided_at: Some(r.decided_at),
                    });
                    if r.passed {
                        passed += 1;
                    } else if r.statistic.is_some() {
                        failed.push(format!("gate {g} ({}) failed: {}", r.gate_name, r.detail));
                    } else {
                        unknown.push(format!(
                            "gate {g} ({}) could not run: {}",
                            r.gate_name, r.detail
                        ));
                    }
                }
            }
        }
        let (verdict, why) = if !failed.is_empty() {
            (StageVerdict::Fail, failed.join("; "))
        } else if !unknown.is_empty() {
            (StageVerdict::Unknown, unknown.join("; "))
        } else {
            (
                StageVerdict::Pass,
                format!("all {passed} gate row(s) passed"),
            )
        };
        stages.push(Stage {
            stage: name,
            verdict,
            rows_read: read,
            why,
        });
    }
    let first_unresolved = stages
        .iter()
        .find(|s| s.verdict != StageVerdict::Pass)
        .map(|s| s.stage);
    let complete = first_unresolved.is_none();
    AlphaChain {
        stages,
        first_unresolved,
        complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn row(gate_no: i32, passed: bool, statistic: Option<f64>, minute: u32) -> GateRow {
        GateRow {
            gate_no,
            gate_name: format!("g{gate_no}"),
            profile_id: "strict_v1".into(),
            passed,
            statistic,
            detail: format!("detail {gate_no}"),
            decided_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, minute, 0).unwrap(),
        }
    }

    fn all_pass() -> Vec<GateRow> {
        [2, 7, 8, 13, 14, 12, 15, 3, 4]
            .iter()
            .map(|g| row(*g, true, Some(1.0), 0))
            .collect()
    }

    fn stage<'a>(c: &'a AlphaChain, name: &str) -> &'a Stage {
        c.stages.iter().find(|s| s.stage == name).unwrap()
    }

    #[test]
    fn an_experiment_with_no_gate_3_or_4_evidence_reports_stage_five_unknown() {
        let rows: Vec<GateRow> = all_pass()
            .into_iter()
            .filter(|r| r.gate_no != 3 && r.gate_no != 4)
            .collect();
        let c = audit(&rows);
        let s5 = stage(&c, "net_alpha_after_costs");
        assert_eq!(s5.verdict, StageVerdict::Unknown);
        assert!(s5.rows_read.iter().all(|r| r.state == "absent"));
        assert!(!c.complete);
        assert_eq!(c.first_unresolved, Some("net_alpha_after_costs"));
        assert_eq!(stage(&c, "pit_information").verdict, StageVerdict::Pass);
    }

    #[test]
    fn no_stage_can_pass_from_an_absent_row_and_nothing_passes_on_no_rows() {
        let c = audit(&[]);
        assert!(c.stages.iter().all(|s| s.verdict == StageVerdict::Unknown));
        assert!(!c.complete);
        // One gate of a multi-gate stage missing is enough to withhold the pass.
        let mut rows = all_pass();
        rows.retain(|r| r.gate_no != 13);
        assert_eq!(
            stage(&audit(&rows), "stable_signal").verdict,
            StageVerdict::Unknown
        );
    }

    #[test]
    fn every_stage_names_the_gate_rows_it_read() {
        let c = audit(&all_pass());
        assert!(c.complete && c.first_unresolved.is_none());
        for s in &c.stages {
            assert!(!s.rows_read.is_empty(), "{}", s.stage);
            assert!(s
                .rows_read
                .iter()
                .all(|r| r.state == "present" && r.profile_id.is_some() && r.decided_at.is_some()));
        }
        assert_eq!(
            stage(&c, "stable_signal")
                .rows_read
                .iter()
                .map(|r| r.gate_no)
                .collect::<Vec<_>>(),
            vec![7, 8, 13, 14]
        );
    }

    #[test]
    fn a_measured_failure_beats_an_unknown_sibling_and_a_run_that_could_not_run_is_unknown() {
        let mut rows = all_pass();
        rows.retain(|r| r.gate_no != 14);
        if let Some(r) = rows.iter_mut().find(|r| r.gate_no == 8) {
            r.passed = false;
        }
        assert_eq!(
            stage(&audit(&rows), "stable_signal").verdict,
            StageVerdict::Fail
        );
        // A failure with no statistic is the stack's record of an inconclusive gate.
        let mut rows = all_pass();
        let g = rows.iter_mut().find(|r| r.gate_no == 15).unwrap();
        g.passed = false;
        g.statistic = None;
        assert_eq!(
            stage(&audit(&rows), "executable_orders").verdict,
            StageVerdict::Unknown
        );
    }

    #[test]
    fn the_newest_row_for_a_gate_is_the_standing_verdict() {
        let mut rows = all_pass();
        rows.push(row(2, false, Some(0.5), 30)); // later: the suite was re-run and failed
        assert_eq!(
            stage(&audit(&rows), "pit_information").verdict,
            StageVerdict::Fail
        );
        let mut rows = all_pass();
        rows.iter_mut().find(|r| r.gate_no == 2).unwrap().passed = false;
        rows.push(row(2, true, Some(1.0), 45)); // later: fixed
        assert_eq!(
            stage(&audit(&rows), "pit_information").verdict,
            StageVerdict::Pass
        );
    }

    #[test]
    fn every_gate_the_table_names_is_one_of_the_sixteen() {
        for (_, gates) in STAGES {
            for g in gates {
                assert!((1..=16).contains(g));
            }
        }
        let mut all: Vec<i32> = STAGES.iter().flat_map(|(_, g)| g.iter().copied()).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 9, "no gate is read by two stages");
    }
}

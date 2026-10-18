//! The execution policy a run states about itself (Set M M-0.4; FEAT-005 §3.5, B1).
//!
//! Every run manifest and every report's execution-assumptions block names the fill model, the
//! fill delay and the fee schedule, so a number is never read without the timing and costs that
//! produced it. A `CurrentClose` fill (decide and fill on the same close) is optimistic by
//! construction, and the verdicts a run feeds say so: [`flags_for`] gives the flag a gate
//! attaches, [`ExecutionPolicy::label`] gives the phrase a report prints.
//!
//! [`ExecutionPolicy::of`] is the one place the block is derived from a `RunConfig`, so the
//! manifest and the report cannot state two different policies for one run.

use serde::{Deserialize, Serialize};

use crate::run::{Flag, FillModel, RunConfig};

/// The flag code a `CurrentClose` run carries into the verdicts it feeds.
pub const OPTIMISTIC_CLOSE: &str = "execution_policy.optimistic_close";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub fill_model: FillModel,
    /// Bars every decided order waits beyond the fill model's own timing.
    pub fill_delay_bars: u32,
    /// What the run was priced under: the config's `cost_model_ref`, which names the fee
    /// schedule as well as the spread and impact legs.
    pub cost_model_ref: String,
    /// `optimistic_close` for a `CurrentClose` run, else `standard`.
    pub label: String,
}

impl ExecutionPolicy {
    #[must_use]
    pub fn of(cfg: &RunConfig) -> Self {
        Self {
            fill_model: cfg.fill_model,
            fill_delay_bars: cfg.fill_delay_bars,
            cost_model_ref: cfg.cost_model_ref.clone(),
            label: if cfg.fill_model == FillModel::CurrentClose { "optimistic_close" } else { "standard" }.into(),
        }
    }

    #[must_use]
    pub fn is_optimistic(&self) -> bool {
        self.fill_model == FillModel::CurrentClose
    }

    /// The execution-assumptions block, as a report prints it.
    #[must_use]
    pub fn block(&self) -> serde_json::Value {
        serde_json::json!({
            "fill_model": self.fill_model,
            "fill_delay_bars": self.fill_delay_bars,
            "cost_model_ref": self.cost_model_ref,
            "label": self.label,
        })
    }
}

/// The flag a gate verdict fed by this run carries, if any.
#[must_use]
pub fn flags_for(cfg: &RunConfig) -> Vec<Flag> {
    if ExecutionPolicy::of(cfg).is_optimistic() {
        vec![Flag {
            code: OPTIMISTIC_CLOSE.into(),
            detail: "the run decides and fills on the same close; its fills are optimistic by construction".into(),
        }]
    } else {
        Vec::new()
    }
}

/// A report's execution-assumptions section. `None` is refused: a report with no policy block
/// is one whose numbers cannot be read.
///
/// # Errors
/// A report body with no `execution_policy` object naming all four fields.
pub fn require_block(report: &serde_json::Value) -> Result<(), String> {
    let block = report.get("execution_policy").ok_or("the report has no execution_policy block")?;
    for k in ["fill_model", "fill_delay_bars", "cost_model_ref", "label"] {
        if block.get(k).is_none() {
            return Err(format!("the execution_policy block has no `{k}`"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{DataSlice, EvalResolution, RunConfigBuilder};
    use chrono::{TimeZone, Utc};

    fn cfg(model: FillModel, delay: u32) -> RunConfig {
        let slice = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
            EvalResolution::Hour1,
        );
        RunConfigBuilder::new("s", "v", slice, "cost:BTC-USD@2025-01-01", "sz", "snap")
            .fill_model(model)
            .fill_delay_bars(delay)
            .build()
    }

    #[test]
    fn a_stored_runs_policy_names_its_fill_model_delay_and_cost_model() {
        let p = ExecutionPolicy::of(&cfg(FillModel::NextBarOpen, 2));
        assert_eq!(p.block()["fill_model"], "next_bar_open");
        assert_eq!(p.block()["fill_delay_bars"], 2);
        assert_eq!(p.block()["cost_model_ref"], "cost:BTC-USD@2025-01-01");
        assert_eq!(p.label, "standard");
        assert!(flags_for(&cfg(FillModel::NextBarOpen, 0)).is_empty());
    }

    #[test]
    fn a_current_close_run_is_labelled_optimistic_close_in_the_verdict_it_feeds() {
        let c = cfg(FillModel::CurrentClose, 0);
        assert_eq!(ExecutionPolicy::of(&c).label, "optimistic_close");
        let f = flags_for(&c);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].code, OPTIMISTIC_CLOSE);
    }

    #[test]
    fn a_report_rendered_without_the_block_is_refused() {
        let with = serde_json::json!({"execution_policy": ExecutionPolicy::of(&cfg(FillModel::NextBarOpen, 0)).block()});
        assert!(require_block(&with).is_ok());
        assert!(require_block(&serde_json::json!({})).is_err());
        assert!(require_block(&serde_json::json!({"execution_policy": {"fill_model": "x"}})).is_err());
    }
}

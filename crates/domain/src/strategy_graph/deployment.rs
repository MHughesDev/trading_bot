//! Deployments: a trading strategy bound to where it trades (FEAT-007 §4).
//!
//! Every trading strategy stays single-asset — the June 2026 "per-asset
//! instance" decision — and a deployment says *which* asset: a fixed instrument,
//! or whatever a selection strategy picks, re-evaluated on its schedule.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::graph::RiskLimits;

/// Where a deployment runs. There is deliberately no `Backtest` here: a backtest
/// is a run, not a deployment, and the type keeps the two from being confused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentEnvironment {
    Paper,
    /// Built and tested; refused by policy until a human enables live trading.
    Live,
}

/// What a deployment trades.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeploymentTarget {
    Instrument {
        instrument: String,
    },
    /// Run the trading strategy on the top `top_n` of a selection.
    Selection {
        /// The selection strategy's revision.
        selection: String,
        top_n: u32,
        #[serde(default)]
        rebalance: Rebalance,
        #[serde(default)]
        on_removal: OnRemoval,
        #[serde(default)]
        weighting: Weighting,
    },
}

/// When the selection is re-run and membership re-applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rebalance {
    /// Whenever the selection strategy runs on its own triggers.
    #[default]
    OnSelectionRun,
    /// On a cron schedule of its own.
    Schedule { cron: String },
}

/// What happens to an instrument that leaves the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnRemoval {
    /// Flatten it now. The default, because holding something the selection no
    /// longer wants is a position nobody decided to keep.
    #[default]
    Close,
    /// Keep the trading instance running until its own rules exit, then stop it.
    LetStrategyExit,
    /// Stop deciding; keep the position as it is.
    Hold,
}

/// How capital is divided across a selection's members.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Weighting {
    #[default]
    Equal,
    /// Proportional to a selection attribute (e.g. `weight`), normalised to sum
    /// to one; members whose attribute is missing get nothing.
    Attribute { name: String },
}

/// A request to deploy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentSpec {
    /// The trading strategy's revision.
    pub trading: String,
    pub target: DeploymentTarget,
    pub environment: DeploymentEnvironment,
    /// Starting capital, in `numeraire`.
    pub capital: Decimal,
    pub numeraire: String,
    /// Tighten-only, on top of the strategy's own and the global limits.
    #[serde(default)]
    pub risk: RiskLimits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_targets_round_trip_with_defaults_spelled_out() {
        for t in [
            serde_json::json!({"kind":"instrument","instrument":"BTC-USD"}),
            serde_json::json!({"kind":"selection","selection":"b".repeat(64),"top_n":5}),
            serde_json::json!({"kind":"selection","selection":"b".repeat(64),"top_n":5,
                               "rebalance":{"kind":"schedule","cron":"0 0 * * 1"},
                               "on_removal":"let_strategy_exit",
                               "weighting":{"kind":"attribute","name":"weight"}}),
        ] {
            let d: DeploymentTarget = serde_json::from_value(t).unwrap();
            let back: DeploymentTarget =
                serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
            assert_eq!(back, d);
        }
        let d: DeploymentTarget = serde_json::from_value(
            serde_json::json!({"kind":"selection","selection":"x","top_n":3}),
        )
        .unwrap();
        let DeploymentTarget::Selection { on_removal, .. } = d else {
            panic!()
        };
        assert_eq!(
            on_removal,
            OnRemoval::Close,
            "leavers are flattened by default"
        );
    }
}

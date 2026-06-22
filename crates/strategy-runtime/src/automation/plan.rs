//! `AutomationPlan` data model — both SingleInstrument and Pipeline shapes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use domain::instrument::AssetClass;

use crate::automation::trigger::TriggerSpec;

/// Whether the automation runs against the paper account or live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAccountMode {
    Paper,
    Live,
}

/// Asset-class-aware time window (24/7 assets may omit start/end).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    /// HH:MM in `timezone`, or `None` for 24/7 assets.
    pub start: Option<String>,
    /// HH:MM in `timezone`, or `None` for 24/7 assets.
    pub end: Option<String>,
    /// IANA timezone string (e.g. `"America/New_York"`, `"UTC"`).
    pub timezone: String,
}

/// One ordered filter stage in a pipeline automation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterStage {
    /// Stable stage label (e.g. `"stage_1"`, `"momentum_filter"`).
    pub stage_id: String,
    /// String ID of the discovery strategy used as the filter for this stage.
    pub strategy_id: String,
    /// Optional human-readable label shown in the pipeline board.
    pub label: Option<String>,
}

/// The execution action triggered when instruments clear all pipeline stages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAction {
    /// String ID of the execution strategy used to place orders.
    pub execution_strategy_id: String,
}

/// The shape-specific payload of an automation plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationSpec {
    /// Runs a single execution strategy on one instrument.
    SingleInstrument {
        asset_class: AssetClass,
        instrument_id: String,
        execution_strategy_id: String,
        time_window: TimeWindow,
        /// What event fires the strategy evaluation. Defaults to the 1m bar close.
        #[serde(default)]
        trigger: TriggerSpec,
    },
    /// Filters a universe through ordered discovery stages then executes.
    Pipeline {
        asset_class: AssetClass,
        /// Full universe of instruments this pipeline considers.
        universe: Vec<String>,
        /// Ordered filter stages — instruments must clear all stages in sequence.
        stages: Vec<FilterStage>,
        execution_action: ExecutionAction,
        /// What event fires each pipeline evaluation pass. Defaults to the 1m bar close.
        #[serde(default)]
        trigger: TriggerSpec,
    },
}

/// A complete automation plan record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationPlan {
    pub id: Uuid,
    pub user_id: Uuid,
    pub account_mode: AutomationAccountMode,
    pub spec: AutomationSpec,
    /// When `true`, the automation is live and will route orders on rising edges.
    pub armed: bool,
    pub created_at: DateTime<Utc>,
}

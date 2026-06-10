//! `AutomationPlan` data model — both SingleInstrument and Pipeline shapes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use domain::instrument::{AssetClass, InstrumentId};

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
    /// UUID of the discovery strategy used as the filter for this stage.
    pub strategy_id: Uuid,
    /// Optional human-readable label shown in the pipeline board.
    pub label: Option<String>,
}

/// The execution action triggered when instruments clear all pipeline stages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAction {
    /// UUID of the execution strategy used to place orders.
    pub execution_strategy_id: Uuid,
}

/// The shape-specific payload of an automation plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationSpec {
    /// Runs a single execution strategy on one instrument.
    SingleInstrument {
        asset_class: AssetClass,
        instrument_id: InstrumentId,
        execution_strategy_id: Uuid,
        time_window: TimeWindow,
    },
    /// Filters a universe through ordered discovery stages then executes.
    Pipeline {
        asset_class: AssetClass,
        /// Full universe of instruments this pipeline considers.
        universe: Vec<InstrumentId>,
        /// Ordered filter stages — instruments must clear all stages in sequence.
        stages: Vec<FilterStage>,
        execution_action: ExecutionAction,
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

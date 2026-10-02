//! The compiled form of a strategy graph: what the engine executes.
//!
//! A plan is a list of nodes in a fixed topological order, each writing one
//! value slot, plus the rules (or selection sink) that read those slots. Every
//! reference has been resolved, every type checked, every parameter substituted,
//! every inline node given a place, and every indicator assigned a column on the
//! input it is computed from. Nothing in a plan is looked up at run time.

use std::collections::BTreeMap;
use std::sync::Arc;

use domain::model_def::contract::IoContract;
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::graph::{InstrumentRef, RiskLimits, SortOrder, Triggers, UniverseRef};
use domain::strategy_graph::intent::TradeIntent;
use domain::strategy_graph::ops::OpSpec;
use domain::strategy_graph::{StrategyKind, TimeframeKey, Value, ValueType};
use serde::Serialize;

/// Index of a node's value slot (= its position in [`Plan::nodes`]).
pub type Slot = usize;

/// Index into [`Plan::inputs`]. Input 0 is always the bound instrument at the
/// decision timeframe.
pub type InputIdx = usize;

pub const PRIMARY: InputIdx = 0;

/// One stream of bars the plan reads, and the indicator columns computed on it.
#[derive(Clone, Debug, Serialize)]
pub struct PlanInput {
    /// `$self` for the primary input, otherwise the declared input's id.
    pub id: String,
    pub instrument: InstrumentRef,
    pub timeframe: TimeframeKey,
    /// DATA-006 indicators computed on every closed bar of this input, in column
    /// order. Computed once per bar, read by every node and model window that
    /// needs them.
    pub columns: Vec<String>,
    /// Bars of history to keep: enough for the longest indicator lookback and
    /// the longest model window over this input.
    pub history_bars: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BarField {
    Open,
    High,
    Low,
    Close,
    Volume,
}

impl BarField {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => Self::Open,
            "high" => Self::High,
            "low" => Self::Low,
            "close" => Self::Close,
            "volume" => Self::Volume,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::High => "high",
            Self::Low => "low",
            Self::Close => "close",
            Self::Volume => "volume",
        }
    }
}

/// A `state.*` field (FEAT-007 §5.4), read from the portfolio view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateField {
    Flat,
    Long,
    Short,
    Position,
    PositionFraction,
    EntryPrice,
    BarsInPosition,
    UnrealizedReturn,
    RealizedPnlToday,
    Equity,
    Cash,
    OpenOrders,
    Mark,
}

impl StateField {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "flat" => Self::Flat,
            "long" => Self::Long,
            "short" => Self::Short,
            "position" => Self::Position,
            "position_fraction" => Self::PositionFraction,
            "entry_price" => Self::EntryPrice,
            "bars_in_position" => Self::BarsInPosition,
            "unrealized_return" => Self::UnrealizedReturn,
            "realized_pnl_today" => Self::RealizedPnlToday,
            "equity" => Self::Equity,
            "cash" => Self::Cash,
            "open_orders" => Self::OpenOrders,
            "mark" => Self::Mark,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn value_type(self) -> ValueType {
        match self {
            Self::Flat | Self::Long | Self::Short => ValueType::Bool,
            _ => ValueType::Num,
        }
    }
}

/// Where a node or rule reads a value from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "of", rename_all = "snake_case")]
pub enum Operand {
    Slot(Slot),
    /// One field of a record-valued slot (a model's output).
    Field(Slot, String),
    /// A literal or a parameter, fixed before the run.
    Const(Value),
    State(StateField),
    /// A closed bar's field on an input.
    Bar(InputIdx, BarField),
    List(Vec<Operand>),
}

/// A model input, assembled by the engine at the decision instant.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelArg {
    /// One row: the named indicator columns on `input`, now.
    Row {
        input: InputIdx,
        columns: Vec<(String, usize)>,
    },
    /// The last `lookback` rows, oldest first.
    Window {
        input: InputIdx,
        columns: Vec<(String, usize)>,
        lookback: u32,
    },
    /// A graph value — another model's output, an indicator, a text.
    Value(Operand),
}

/// A call to a pinned model.
#[derive(Clone, Debug, Serialize)]
pub struct ModelCall {
    /// As written in the strategy (`vol_forecaster@12`).
    pub reference: String,
    pub model: String,
    pub version: u32,
    #[serde(skip)]
    pub contract: Arc<IoContract>,
    pub inputs: BTreeMap<String, ModelArg>,
}

/// What a node computes.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    /// A DATA-006 indicator column on an input.
    Feature {
        input: InputIdx,
        column: usize,
        name: String,
        lookback: u32,
    },
    Bar {
        input: InputIdx,
        field: BarField,
    },
    Hour,
    Weekday,
    /// An operator from the registry, with its resolved arguments by port name.
    /// `switch`'s case labels are further entries.
    Op {
        #[serde(serialize_with = "op_name")]
        op: &'static OpSpec,
        args: BTreeMap<String, Operand>,
    },
    Model(ModelCall),
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn op_name<S: serde::Serializer>(op: &&'static OpSpec, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(op.name)
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanNode {
    /// The graph's node id; inline nodes get `parent~port`.
    pub id: String,
    pub kind: NodeKind,
    pub ty: ValueType,
    /// Decision ticks before this node can first be present.
    pub warmup: u32,
}

impl PlanNode {
    #[must_use]
    pub const fn is_model(&self) -> bool {
        matches!(self.kind, NodeKind::Model(_))
    }
}

/// One rule, compiled.
#[derive(Clone, Debug, Serialize)]
pub struct CompiledRule {
    pub index: usize,
    pub name: Option<String>,
    pub group: String,
    pub when: Operand,
    /// The intent with every non-literal number replaced by `#k`, where `k`
    /// indexes `operands`.
    pub template: TradeIntent,
    pub operands: Vec<Operand>,
}

/// The selection sink, compiled.
#[derive(Clone, Debug, Serialize)]
pub struct CompiledSelect {
    pub score: Operand,
    pub filter: Option<Operand>,
    pub top_n: Option<u32>,
    pub order: SortOrder,
    pub attributes: BTreeMap<String, Operand>,
    pub universe: UniverseRef,
}

/// A compiled strategy.
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub kind: StrategyKind,
    pub name: String,
    /// The graph's revision hash.
    pub revision: String,
    pub timeframe: TimeframeKey,
    pub triggers: Triggers,
    pub params: ParamValues,
    pub inputs: Vec<PlanInput>,
    /// In evaluation order.
    pub nodes: Vec<PlanNode>,
    pub rules: Vec<CompiledRule>,
    pub select: Option<CompiledSelect>,
    /// ATR columns on the primary input that intent distances read, by period.
    pub atr_columns: BTreeMap<u32, usize>,
    /// Decision ticks of warm-up the plan needs before every node can be present.
    pub warmup_ticks: u32,
    pub risk: RiskLimits,
}

impl Plan {
    #[must_use]
    pub fn node(&self, id: &str) -> Option<(Slot, &PlanNode)> {
        self.nodes.iter().enumerate().find(|(_, n)| n.id == id)
    }

    /// Bars of history each input needs before the first live decision: the
    /// primary input needs its indicator history *and* the plan's tick warm-up
    /// (its bars are the ticks); every other input needs its indicator history.
    #[must_use]
    pub fn required_history(&self) -> Vec<(InputIdx, u32)> {
        self.inputs
            .iter()
            .enumerate()
            .map(|(i, inp)| {
                let bars = if i == PRIMARY {
                    inp.history_bars.max(self.warmup_ticks)
                } else {
                    inp.history_bars
                };
                (i, bars)
            })
            .collect()
    }
}

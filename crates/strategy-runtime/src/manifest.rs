//! Capability manifest compiler.
//!
//! At save time the backend calls `compile_manifest` to derive a
//! `CapabilityManifest` from the strategy definition graph.  The manifest is
//! persisted in `strategy_manifests` and used for apply-list filtering.

use std::collections::HashSet;

use domain::data_type::DataType;
use domain::strategy_def::{nodes::NodeKind, StrategyDefinition};
use serde::{Deserialize, Serialize};

use crate::kind::{infer_kind, StrategyKind};

/// What triggers an evaluation of this strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationTrigger {
    /// Fires on each 1-minute bar close (default for OHLCV-based strategies).
    BarClose,
    /// Fires on each individual trade tick.
    Tick,
    /// Fires on each NBBO/top-of-book quote update.
    Quote,
    /// Fires on a domain-level event (prediction market, DEX quote, etc.).
    Event,
    /// Fires on a wall-clock schedule.
    Scheduled,
}

impl EvaluationTrigger {
    /// Lowercase string key — avoids `format!("{:?}", trigger).to_lowercase()` allocations.
    pub fn as_str(self) -> &'static str {
        match self {
            EvaluationTrigger::BarClose => "bar_close",
            EvaluationTrigger::Tick => "tick",
            EvaluationTrigger::Quote => "quote",
            EvaluationTrigger::Event => "event",
            EvaluationTrigger::Scheduled => "scheduled",
        }
    }
}

/// Compiled capability manifest — stored alongside the strategy definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityManifest {
    /// Source `DataType` lanes the strategy requires (e.g. `[market.ohlcv]`).
    pub required_lanes: Vec<DataType>,
    /// Required data primitives (currently always empty in v1.0/v1.5).
    pub required_primitives: Vec<String>,
    /// Named derived features the strategy reads (e.g. `["ema_7", "ema_21"]`).
    pub required_features: Vec<String>,
    /// What event drives re-evaluation.
    pub evaluation_trigger: EvaluationTrigger,
    /// Inferred kind — never declared on the definition.
    pub strategy_kind: StrategyKind,
}

// ── Lane → DataType mapping ───────────────────────────────────────────────────

fn lane_to_data_type(lane: &str) -> Option<DataType> {
    if lane.starts_with("market.bars.") || lane == "market.ohlcv" {
        Some(DataType::MarketOhlcv)
    } else if lane == "market.trade" {
        Some(DataType::MarketTrade)
    } else if lane == "market.quote" {
        Some(DataType::MarketQuote)
    } else if lane == "market.funding_rate" {
        Some(DataType::MarketFundingRate)
    } else if lane == "market.open_interest" {
        Some(DataType::MarketOpenInterest)
    } else if lane == "prediction.price" {
        Some(DataType::PredictionMarketPrice)
    } else if lane == "dex.quote" {
        Some(DataType::DexQuote)
    } else if lane == "social.post" {
        Some(DataType::SocialPost)
    } else if lane == "web.page_snapshot" {
        Some(DataType::WebPageSnapshot)
    } else if lane == "news.article" {
        Some(DataType::NewsArticle)
    } else {
        // Derived lanes (features.technical, etc.) don't add source requirements.
        None
    }
}

fn infer_trigger(lanes: &[DataType]) -> EvaluationTrigger {
    if lanes.contains(&DataType::MarketOhlcv) {
        EvaluationTrigger::BarClose
    } else if lanes.contains(&DataType::MarketTrade) {
        EvaluationTrigger::Tick
    } else if lanes.contains(&DataType::MarketQuote) {
        EvaluationTrigger::Quote
    } else {
        EvaluationTrigger::Event
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Compile a capability manifest from a strategy definition.
///
/// Walks the `inputs` and any `DataSource` nodes to collect `required_lanes`.
/// Collects named features from `inputs` with a `features` list.
/// Infers the evaluation trigger and strategy kind.
pub fn compile_manifest(def: &StrategyDefinition) -> CapabilityManifest {
    let mut seen_lanes: HashSet<DataType> = HashSet::new();
    let mut required_lanes: Vec<DataType> = Vec::new();

    // Source lanes from InputDeclaration entries.
    for input in &def.inputs {
        if let Some(dt) = lane_to_data_type(&input.lane) {
            if seen_lanes.insert(dt) {
                required_lanes.push(dt);
            }
        }
    }

    // Source lanes from v1.5 DataSource nodes.
    for node in &def.nodes {
        if let NodeKind::DataSource { data_type } = &node.kind {
            if let Ok(dt) = data_type.parse::<DataType>() {
                if seen_lanes.insert(dt) {
                    required_lanes.push(dt);
                }
            }
        }
    }

    // Named features from inputs.
    // Using a HashSet<&str> for the dedup check avoids cloning each feature string
    // twice (once for the set, once for the vec).  We only clone on insert.
    let mut seen_features: HashSet<&str> = HashSet::new();
    let mut required_features: Vec<String> = Vec::new();
    for input in &def.inputs {
        for feature in &input.features {
            if seen_features.insert(feature.as_str()) {
                required_features.push(feature.clone());
            }
        }
    }

    let evaluation_trigger = infer_trigger(&required_lanes);
    let strategy_kind = infer_kind(def);

    CapabilityManifest {
        required_lanes,
        required_primitives: vec![],
        required_features,
        evaluation_trigger,
        strategy_kind,
    }
}

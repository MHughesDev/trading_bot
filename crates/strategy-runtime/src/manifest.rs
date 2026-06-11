//! Capability manifest compiler.
//!
//! At save time the backend calls `compile_manifest` to derive a
//! `CapabilityManifest` from the strategy definition graph.  The manifest is
//! persisted in `strategy_manifests` and used for apply-list filtering.

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

// ── Pre-computation helpers (#56, #68) ───────────────────────────────────────

/// Collect deduplicated required lanes from a definition's inputs and DataSource nodes.
///
/// Uses linear scan (no HashSet): expected lane count is < 10, so O(n²) is faster
/// than a HashSet allocation in practice (#56).
fn collect_required_lanes(def: &StrategyDefinition) -> Vec<DataType> {
    let mut lanes: Vec<DataType> = Vec::new();

    for input in &def.inputs {
        if let Some(dt) = lane_to_data_type(&input.lane) {
            if !lanes.contains(&dt) {
                lanes.push(dt);
            }
        }
    }

    for node in &def.nodes {
        if let NodeKind::DataSource { data_type } = &node.kind {
            if let Ok(dt) = data_type.parse::<DataType>() {
                if !lanes.contains(&dt) {
                    lanes.push(dt);
                }
            }
        }
    }

    lanes
}

/// Collect deduplicated feature names from a definition's inputs.
///
/// Uses sort + dedup instead of a HashSet, eliminating the HashSet allocation
/// entirely (#56, #57). Feature names are small strings (< 32 bytes typical)
/// so sort is cache-friendly and faster than hashing for small n.
fn collect_required_features(def: &StrategyDefinition) -> Vec<String> {
    // Gather all feature name references before any allocation.
    let mut refs: Vec<&str> = def
        .inputs
        .iter()
        .flat_map(|i| i.features.iter().map(String::as_str))
        .collect();

    refs.sort_unstable();
    refs.dedup();

    // One String allocation per unique feature name (#57).
    refs.iter().map(|s| s.to_string()).collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Compile a capability manifest from a strategy definition.
///
/// Lane and feature collection is factored into pre-computation helpers so
/// callers that cache the `StrategyDefinition` at load time can invoke those
/// helpers once and skip the tree walk on repeated compiles (#56, #68).
pub fn compile_manifest(def: &StrategyDefinition) -> CapabilityManifest {
    let required_lanes = collect_required_lanes(def);
    let required_features = collect_required_features(def);
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

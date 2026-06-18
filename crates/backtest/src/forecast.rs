//! Forecast provider seam for model-driven backtests.
//!
//! `ModelForecast` strategy nodes need a per-bar boolean (did the model's
//! forecast match the node's direction/confidence?).  Computing that requires
//! the inference gateway, which lives in `model-registry` — a crate that
//! depends on `backtest`.  To avoid a dependency cycle, `backtest` defines this
//! trait and `model-registry` implements it; the platform injects a concrete
//! provider into [`crate::BacktestManager`].
//!
//! When no provider is wired (or the definition has no model nodes), backtests
//! run exactly as before and model nodes abstain (never fire).

use std::collections::HashMap;

use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;

use crate::requirements::FeatureSpec;
use crate::store::LoadedBar;

/// Produces per-bar forecast outcomes for every `ModelForecast` node in a
/// definition.  Implemented in `model-registry` over the live inference
/// gateway; called by the `BacktestManager` in async land before the blocking
/// simulation begins.
#[async_trait::async_trait]
pub trait ForecastProvider: Send + Sync {
    /// Returns `node_id → one boolean per bar` (aligned to `bars` order) for
    /// each `ModelForecast` node.  A node absent from the map — or a provider
    /// that returns an empty map — means that node abstains for the whole run.
    async fn forecasts(
        &self,
        definition: &StrategyDefinition,
        instrument_id: &str,
        timeframe: Timeframe,
        bars: &[LoadedBar],
        features: &[FeatureSpec],
    ) -> HashMap<String, Vec<bool>>;
}

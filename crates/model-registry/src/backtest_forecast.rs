//! Backtest forecast provider — implements [`backtest::ForecastProvider`] over
//! the live [`InferenceGateway`] so `ModelForecast` strategy nodes evaluate in
//! historical simulation.
//!
//! For each bar we recompute the strategy's indicator features (the same EMA/RSI
//! primitives the sim handler uses) and ask the gateway whether each model node
//! fires (direction match + confidence ≥ threshold).  Inference is async and
//! runs here, before the blocking simulation.
//!
//! Limitation (Stage 1): the features passed are the strategy's *indicator*
//! features, not necessarily the forecaster's full training feature set. A model
//! that needs features the strategy doesn't declare will see a sparse vector and
//! likely abstain. Computing the model's own feature pipeline per bar is a
//! follow-up.

use std::collections::HashMap;
use std::sync::Arc;

use backtest::forecast::ForecastProvider;
use backtest::requirements::{FeatureKind, FeatureSpec};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use rust_decimal::prelude::ToPrimitive;

use crate::inference_gateway::InferenceGateway;

enum IndicatorState {
    Ema(features::Ema),
    Rsi(features::Rsi),
}

/// Concrete [`ForecastProvider`] backed by the inference gateway.
pub struct GatewayForecastProvider {
    gateway: Arc<InferenceGateway>,
}

impl GatewayForecastProvider {
    pub fn new(gateway: Arc<InferenceGateway>) -> Arc<Self> {
        Arc::new(Self { gateway })
    }
}

#[async_trait::async_trait]
impl ForecastProvider for GatewayForecastProvider {
    async fn forecasts(
        &self,
        definition: &StrategyDefinition,
        instrument_id: &str,
        _timeframe: Timeframe,
        bars: &[LoadedBar],
        features: &[FeatureSpec],
    ) -> HashMap<String, Vec<bool>> {
        // Per-bar incremental feature state, mirroring backtest/src/sim.rs.
        let mut indicators: Vec<(String, IndicatorState)> = features
            .iter()
            .map(|f| {
                let state = match f.kind {
                    FeatureKind::Ema => IndicatorState::Ema(features::Ema::new(f.period)),
                    FeatureKind::Rsi => IndicatorState::Rsi(features::Rsi::new(f.period)),
                };
                (f.name.clone(), state)
            })
            .collect();

        let mut out: HashMap<String, Vec<bool>> = HashMap::new();
        let mut feature_values: HashMap<String, f64> = HashMap::new();

        for bar in bars {
            let close = bar.close.to_f64().unwrap_or(0.0);
            for (name, state) in &mut indicators {
                let v = match state {
                    IndicatorState::Ema(ema) => Some(ema.update(close)),
                    IndicatorState::Rsi(rsi) => rsi.update(close),
                };
                if let Some(v) = v {
                    feature_values.insert(name.clone(), v);
                }
            }

            let mut results: HashMap<String, bool> = HashMap::new();
            self.gateway
                .refresh_node_forecasts(
                    &definition.nodes,
                    instrument_id,
                    &feature_values,
                    &mut results,
                )
                .await;

            for (node_id, fired) in results {
                out.entry(node_id).or_default().push(fired);
            }
        }

        out
    }
}

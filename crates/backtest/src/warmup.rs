//! Scanner / live-session indicator warmup.
//!
//! When a scanner panel or live strategy session selects a timeframe other
//! than 1 min, the indicators need historical data to converge before the
//! first evaluation.  This module answers the question: "how much history do
//! I need, and how do I get warm indicator values from it?"
//!
//! # Algorithm
//!
//! 1. Derive the required warmup bar count from the strategy's indicator
//!    periods (via [`DataRequirements::warmup_bars`]).
//! 2. Compute how many 1 min bars that translates to for the chosen
//!    timeframe, adding a small buffer for partial buckets and market gaps.
//! 3. Load those 1 min bars from `ClickHouse`.
//! 4. Aggregate in-memory to the target timeframe ([`aggregate_bars`]).
//! 5. Run all indicators over every aggregated bar, returning the final
//!    (warm) state.
//!
//! The returned [`WarmState`] carries both the indicator values (ready for
//! the first live evaluation) and the aggregated bars (so the caller can
//! seed its in-progress-bar accumulator without a second DB round-trip).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;

use crate::aggregate::aggregate_bars;
use crate::requirements::{DataRequirements, FeatureKind};
use crate::store::{BarStore, LoadedBar};
use crate::types::TimeframeExt;

/// Extra target-timeframe bars to request beyond the warmup floor.
///
/// Absorbs partial leading/trailing buckets caused by session start time not
/// aligning to a bucket boundary, and fills in gaps from market closures.
const BUFFER_BARS: u64 = 5;

/// Warm indicator state produced by [`load_warm_state`].
#[derive(Debug, Default)]
pub struct WarmState {
    /// Last computed value for each feature name (e.g. `"ema_7"`).
    ///
    /// Features that have not yet emitted a value (RSI before period+1 bars
    /// of aggregated history exist) are absent — callers should treat a
    /// missing value as "not yet ready" and skip evaluation until it appears.
    pub values: HashMap<String, f64>,

    /// Aggregated target-timeframe bars that were used to warm the indicators,
    /// ordered ascending by `ts_ns`.
    ///
    /// The caller can use this to seed its in-progress bar accumulator:
    /// the last bar in the slice is the most recently *closed* target bar.
    pub bars: Vec<LoadedBar>,
}

/// Loads 1 min bars from `ClickHouse`, aggregates them to `target_timeframe`,
/// and runs all indicators in `requirements` over the full history to produce
/// a warm [`WarmState`].
///
/// `as_of` is the upper bound of the load window — typically `Utc::now()` for
/// a live scanner session, or the replay clock for deterministic testing.
///
/// Returns an empty `WarmState` when the strategy has no indicator inputs.
pub async fn load_warm_state(
    store: &BarStore,
    instrument_id: &str,
    target_timeframe: Timeframe,
    requirements: &DataRequirements,
    as_of: DateTime<Utc>,
) -> anyhow::Result<WarmState> {
    if requirements.features.is_empty() {
        return Ok(WarmState::default());
    }

    let target_secs = target_timeframe.seconds();
    let base_secs = Timeframe::Minutes1.seconds(); // 60

    anyhow::ensure!(
        target_secs % base_secs == 0,
        "target timeframe {target_timeframe:?} ({target_secs}s) is not a whole multiple of 1 min",
    );

    // How many 1 min base bars do we need?
    let bars_needed = requirements.warmup_bars + BUFFER_BARS;
    let base_bars_needed = bars_needed * (target_secs / base_secs);
    let window_secs = i64::try_from(base_bars_needed * base_secs).unwrap_or(i64::MAX / 2);
    let data_from = as_of - Duration::seconds(window_secs);

    let base_bars = store
        .load_bars(instrument_id, Timeframe::Minutes1, data_from, as_of)
        .await
        .map_err(|e| anyhow::anyhow!("warmup bar load failed for {instrument_id}: {e}"))?;

    let aggregated = aggregate_bars(&base_bars, target_timeframe, Timeframe::Minutes1);

    let values = run_indicators(&aggregated, requirements);

    Ok(WarmState {
        values,
        bars: aggregated,
    })
}

/// Drives all indicators in `requirements` over `bars` (ascending) and
/// returns the last emitted value for each feature name.
pub fn run_indicators(bars: &[LoadedBar], requirements: &DataRequirements) -> HashMap<String, f64> {
    enum IndicatorState {
        Ema(features::Ema),
        Rsi(features::Rsi),
    }

    let mut indicators: Vec<(String, IndicatorState)> = requirements
        .features
        .iter()
        .map(|f| {
            let state = match f.kind {
                FeatureKind::Ema => IndicatorState::Ema(features::Ema::new(f.period)),
                FeatureKind::Rsi => IndicatorState::Rsi(features::Rsi::new(f.period)),
            };
            (f.name.clone(), state)
        })
        .collect();

    let mut values: HashMap<String, f64> = HashMap::new();

    for bar in bars {
        let close = bar.close.to_f64().unwrap_or(0.0);
        for (name, state) in &mut indicators {
            let value = match state {
                IndicatorState::Ema(ema) => Some(ema.update(close)),
                IndicatorState::Rsi(rsi) => rsi.update(close),
            };
            if let Some(v) = value {
                values.insert(name.clone(), v);
            }
        }
    }

    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requirements::{FeatureKind, FeatureSpec};
    use rust_decimal_macros::dec;

    fn rising_bars(n: usize) -> Vec<LoadedBar> {
        (1..=n)
            .map(|i| LoadedBar {
                ts_ns: i as i64 * 60_000_000_000,
                open: dec!(100),
                high: dec!(100),
                low: dec!(100),
                close: rust_decimal::Decimal::from(100 + i as i64),
                volume: dec!(1),
                trade_count: 1,
            })
            .collect()
    }

    fn ema_requirements(period: usize) -> DataRequirements {
        DataRequirements {
            timeframe: Timeframe::Minutes1,
            features: vec![FeatureSpec {
                name: format!("ema_{period}"),
                kind: FeatureKind::Ema,
                period,
            }],
            warmup_bars: (period as u64) * 5,
        }
    }

    #[test]
    fn ema_value_is_present_after_one_bar() {
        let bars = rising_bars(1);
        let req = ema_requirements(7);
        let values = run_indicators(&bars, &req);
        assert!(values.contains_key("ema_7"), "EMA seeds on first bar");
    }

    #[test]
    fn rsi_absent_until_period_plus_one_bars() {
        let req = DataRequirements {
            timeframe: Timeframe::Minutes1,
            features: vec![FeatureSpec {
                name: "rsi_3".to_string(),
                kind: FeatureKind::Rsi,
                period: 3,
            }],
            warmup_bars: 30,
        };
        // 3 bars → not enough (needs period + 1 = 4)
        let short = rising_bars(3);
        assert!(!run_indicators(&short, &req).contains_key("rsi_3"));

        // 4 bars → first RSI value available
        let enough = rising_bars(4);
        assert!(run_indicators(&enough, &req).contains_key("rsi_3"));
    }

    #[test]
    fn ema_converges_toward_rising_prices() {
        let bars = rising_bars(50);
        let req = ema_requirements(7);
        let values = run_indicators(&bars, &req);
        let ema = values["ema_7"];
        // After 50 rising bars, EMA(7) should be between 100 and 150
        assert!(
            ema > 100.0 && ema < 155.0,
            "EMA out of expected range: {ema}"
        );
    }

    #[test]
    fn empty_features_returns_empty_map() {
        let req = DataRequirements {
            timeframe: Timeframe::Minutes1,
            features: vec![],
            warmup_bars: 0,
        };
        assert!(run_indicators(&rising_bars(10), &req).is_empty());
    }
}

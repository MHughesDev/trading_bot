//! Bridge between platform strategy definitions and the `market_simulator` SDK.
//!
//! The platform side owns: the strategy definition (interpreted with the same
//! pure `strategy-runtime` evaluator that runs live), the indicator
//! computation (same pure `features` crate), and the bar data.  The simulator
//! side owns only execution: per-asset-class venue simulation, order
//! matching, fills, fees, and result statistics.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use domain::order::Side;
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::actions::{ActionKind, SizeMode};
use domain::strategy_def::StrategyDefinition;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use nautilus_backtest::sdk::{
    self, BarHandler, BarSimulationSpec, SimOrderCommand, SimulationControl, VenuePreset,
};
use nautilus_core::UnixNanos;
use nautilus_model::data::{Bar, BarSpecification, BarType};
use nautilus_model::enums::{AggregationSource, BarAggregation, OrderSide, PriceType};
use nautilus_model::identifiers::Venue;
use nautilus_model::instruments::Instrument;
use nautilus_model::types::{Money, Price, Quantity};

/// Price/size decimal precision for the simulated instrument.
///
/// When sourced from instrument metadata these are the venue's real tick/lot
/// precisions (so a 0-dp JPY-style or 8-dp crypto instrument quantizes
/// correctly); when absent, [`run_simulation`] infers them from the data.
#[derive(Clone, Copy, Debug)]
pub struct InstrumentPrecisions {
    pub price: u8,
    pub size: u8,
}

/// Non-panicking nautilus value constructors.  Malformed input returns an
/// error instead of panicking inside `spawn_blocking` (#21).
fn price(s: &str) -> anyhow::Result<Price> {
    Price::from_str(s).map_err(|e| anyhow::anyhow!("invalid price '{s}': {e}"))
}
fn quantity(s: &str) -> anyhow::Result<Quantity> {
    Quantity::from_str(s).map_err(|e| anyhow::anyhow!("invalid quantity '{s}': {e}"))
}
fn money(s: &str) -> anyhow::Result<Money> {
    Money::from_str(s).map_err(|e| anyhow::anyhow!("invalid money '{s}': {e}"))
}

use crate::requirements::{FeatureKind, FeatureSpec};
use crate::store::LoadedBar;

/// Everything needed to run one simulation.
#[derive(Clone, Debug)]
pub struct SimulationInputs {
    pub definition: StrategyDefinition,
    pub instrument_id: String,
    pub venue_id: String,
    pub asset_class: String,
    pub timeframe: Timeframe,
    pub quote_currency: String,
    /// Decimal — never a float.
    pub initial_balance: Decimal,
    /// Real venue precisions from instrument metadata; `None` falls back to
    /// inferring precision from the data (#8).
    pub precisions: Option<InstrumentPrecisions>,
    /// Orders are suppressed for bars before this timestamp (warm-up lead-in).
    pub sim_start_ns: i64,
    pub bars: Vec<LoadedBar>,
    pub features: Vec<FeatureSpec>,
}

/// Simulation outcome: the simulator's result document plus run flags.
#[derive(Clone, Debug)]
pub struct SimulationReport {
    pub cancelled: bool,
    pub result: serde_json::Value,
}

enum IndicatorState {
    Ema(features::Ema),
    Rsi(features::Rsi),
}

/// Formats a `Decimal` with exactly `precision` fractional digits.
fn dec_str(d: Decimal, precision: u32) -> String {
    format!("{:.*}", precision as usize, d.round_dp(precision))
}

fn max_scale(values: impl Iterator<Item = Decimal>) -> u32 {
    values.map(|v| v.normalize().scale()).max().unwrap_or(0)
}

fn timeframe_spec(tf: Timeframe) -> BarSpecification {
    let (step, aggregation) = match tf {
        Timeframe::Seconds1 => (1, BarAggregation::Second),
        Timeframe::Minutes1 => (1, BarAggregation::Minute),
        Timeframe::Minutes5 => (5, BarAggregation::Minute),
        Timeframe::Minutes15 => (15, BarAggregation::Minute),
        Timeframe::Hours1 => (1, BarAggregation::Hour),
        Timeframe::Hours4 => (4, BarAggregation::Hour),
        Timeframe::Daily => (1, BarAggregation::Day),
    };
    BarSpecification::new(step, aggregation, PriceType::Last)
}

/// Runs the simulation synchronously (call from a blocking task).
///
/// Progress and cancellation are exposed through `control`, which the caller
/// polls from async land while this runs.
#[allow(clippy::needless_pass_by_value)] // caller hands off ownership of the bar buffer to the blocking task
pub fn run_simulation(
    inputs: SimulationInputs,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<SimulationReport> {
    anyhow::ensure!(!inputs.bars.is_empty(), "no bars to simulate");

    let order_specs = order_specs(&inputs.definition)?;

    // Precision comes from the instrument's real tick/lot metadata when known
    // (so JPY-style 0-dp and 8-dp crypto instruments quantize correctly).
    // Without metadata it is inferred from the data and order sizes; the engine
    // rejects mismatched precisions, so everything is quantized on the way in.
    let (price_precision, size_precision) = if let Some(p) = inputs.precisions {
        (p.price, p.size)
    } else {
        let price_precision = max_scale(
            inputs
                .bars
                .iter()
                .flat_map(|b| [b.open, b.high, b.low, b.close].into_iter()),
        )
        .clamp(1, 9) as u8;
        let size_precision = max_scale(
            inputs
                .bars
                .iter()
                .map(|b| b.volume)
                .chain(order_specs.iter().map(|(_, _, s)| *s)),
        )
        .clamp(1, 9) as u8;
        (price_precision, size_precision)
    };

    // Simulated venue + instrument per asset class.
    let venue = Venue::from(inputs.venue_id.to_uppercase().as_str());
    let preset = VenuePreset::from_asset_class(&inputs.asset_class);
    let price_increment = price(&increment_str(price_precision))?;
    let instrument = if preset == VenuePreset::Equity {
        sdk::equity_instrument(
            venue,
            &inputs.instrument_id,
            &inputs.quote_currency,
            price_precision,
            price_increment,
        )
    } else {
        let (base, quote) = split_pair(&inputs.instrument_id, &inputs.quote_currency);
        let size_increment = quantity(&increment_str(size_precision))?;
        sdk::spot_pair_instrument(
            venue,
            &inputs.instrument_id,
            &base,
            &quote,
            price_precision,
            size_precision,
            price_increment,
            size_increment,
        )
    };

    let bar_type = BarType::new(
        instrument.id(),
        timeframe_spec(inputs.timeframe),
        AggregationSource::External,
    );

    // Account currency must exist in the simulator's currency registry.
    let _ = sdk::currency_or_register(&inputs.quote_currency, price_precision);
    let starting = money(&format!(
        "{} {}",
        dec_str(inputs.initial_balance, 2),
        inputs.quote_currency
    ))?;

    let engine_bars = to_engine_bars(&inputs.bars, bar_type, price_precision, size_precision)?;

    let spec = BarSimulationSpec {
        venue,
        preset,
        instrument,
        starting_balances: vec![starting],
        bar_types: vec![bar_type],
        chunk_size: chunk_size_for(engine_bars.len()),
    };

    let handler = build_handler(&inputs, order_specs, size_precision)?;
    let outcome = sdk::run_bar_simulation(spec, engine_bars, handler, control)?;

    Ok(SimulationReport {
        cancelled: outcome.cancelled,
        result: serde_json::to_value(&outcome.result)?,
    })
}

/// Chunk size balancing progress granularity against per-chunk overhead.
fn chunk_size_for(total: usize) -> usize {
    (total / 100).clamp(250, 25_000)
}

/// Smallest representable step at `precision` fractional digits ("0.001" etc.).
fn increment_str(precision: u8) -> String {
    if precision == 0 {
        "1".to_string()
    } else {
        format!("0.{}1", "0".repeat(precision as usize - 1))
    }
}

fn split_pair(instrument_id: &str, fallback_quote: &str) -> (String, String) {
    for sep in ['-', '/'] {
        if let Some((base, quote)) = instrument_id.split_once(sep) {
            if !base.is_empty() && !quote.is_empty() {
                return (base.to_string(), quote.to_string());
            }
        }
    }
    (instrument_id.to_string(), fallback_quote.to_string())
}

/// Parses `(signal, side, size)` triples from the definition's actions.
fn order_specs(def: &StrategyDefinition) -> anyhow::Result<Vec<(String, Side, Decimal)>> {
    let mut out = Vec::new();
    for action in &def.actions {
        let ActionKind::PlaceOrder { order } = &action.kind;
        anyhow::ensure!(
            order.size_mode == SizeMode::Fixed,
            "backtest supports fixed-size orders only (v1.0); action on '{}' uses {:?}",
            action.on_signal,
            order.size_mode
        );
        let size: Decimal = order
            .size
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid order size '{}': {e}", order.size))?;
        anyhow::ensure!(
            size > Decimal::ZERO,
            "order size must be positive on action '{}'",
            action.on_signal
        );
        out.push((action.on_signal.clone(), order.side, size));
    }
    Ok(out)
}

fn to_engine_bars(
    bars: &[LoadedBar],
    bar_type: BarType,
    price_precision: u8,
    size_precision: u8,
) -> anyhow::Result<Vec<Bar>> {
    let pp = u32::from(price_precision);
    let sp = u32::from(size_precision);
    bars.iter()
        .map(|b| {
            let ts = UnixNanos::from(u64::try_from(b.ts_ns).unwrap_or(0));
            Ok(Bar::new(
                bar_type,
                price(&dec_str(b.open, pp))?,
                price(&dec_str(b.high, pp))?,
                price(&dec_str(b.low, pp))?,
                price(&dec_str(b.close, pp))?,
                quantity(&dec_str(b.volume.abs(), sp))?,
                ts,
                ts,
            ))
        })
        .collect()
}

/// Builds the bar handler that interprets the strategy definition.
///
/// Per bar: update indicators (pure `features` crate), evaluate the node
/// graph (pure `strategy-runtime` interpreter), and emit orders for signals
/// on their rising edge — a signal must clear and re-fire before it places
/// another order, which is the crossover semantics live strategies get from
/// event-driven dispatch.
fn build_handler(
    inputs: &SimulationInputs,
    order_specs: Vec<(String, Side, Decimal)>,
    size_precision: u8,
) -> anyhow::Result<BarHandler> {
    let nodes = inputs.definition.nodes.clone();
    let timeframe = inputs.timeframe;
    let sim_start_ns = inputs.sim_start_ns;

    // Pre-quantize each order's size to a nautilus `Quantity` once, up front, so
    // a malformed size is an error here rather than a panic inside the per-bar
    // callback (which the engine runs on the blocking pool).
    let order_specs: Vec<(String, OrderSide, Quantity)> = order_specs
        .into_iter()
        .map(|(signal, side, size)| {
            let qty = quantity(&dec_str(size, u32::from(size_precision)))?;
            let order_side = match side {
                Side::Buy => OrderSide::Buy,
                Side::Sell => OrderSide::Sell,
            };
            Ok((signal, order_side, qty))
        })
        .collect::<anyhow::Result<_>>()?;

    let mut indicators: Vec<(String, IndicatorState)> = inputs
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

    let mut feature_values: HashMap<String, f64> = HashMap::new();
    let mut bar_map: HashMap<Timeframe, BarPayload> = HashMap::new();
    let mut active_signals: HashSet<String> = HashSet::new();

    Ok(Box::new(move |bar: &Bar| {
        let close_value = bar.close.as_decimal();

        // Indicators consume the close (same convention as the live feature
        // pipeline).
        let close_input = close_value.to_f64().unwrap_or(0.0);
        for (name, state) in &mut indicators {
            match state {
                IndicatorState::Ema(ema) => {
                    feature_values.insert(name.clone(), ema.update(close_input));
                }
                IndicatorState::Rsi(rsi) => {
                    if let Some(v) = rsi.update(close_input) {
                        feature_values.insert(name.clone(), v);
                    }
                }
            }
        }

        // Materialize the bar for `bar('field')` expressions.  The frozen
        // v1.0 grammar reads bar fields from the 1m lane specifically, so the
        // payload is registered under both the actual timeframe and 1m.
        let payload = BarPayload::new(
            timeframe,
            domain::money::Price::from_decimal(bar.open.as_decimal()),
            domain::money::Price::from_decimal(bar.high.as_decimal()),
            domain::money::Price::from_decimal(bar.low.as_decimal()),
            domain::money::Price::from_decimal(close_value),
            domain::money::Size::from_decimal(bar.volume.as_decimal()),
            0,
        );
        bar_map.insert(timeframe, payload.clone());
        bar_map.insert(Timeframe::Minutes1, payload);

        let fired = strategy_runtime::evaluate_signals(&nodes, &feature_values, &bar_map);
        let fired: HashSet<String> = fired.into_iter().collect();

        let mut commands = Vec::new();
        let in_window = i64::try_from(bar.ts_event.as_u64()).unwrap_or(i64::MAX) >= sim_start_ns;
        if in_window {
            for signal in fired.difference(&active_signals) {
                for (on_signal, side, qty) in &order_specs {
                    if on_signal == signal {
                        commands.push(SimOrderCommand::Market {
                            side: *side,
                            quantity: *qty,
                        });
                    }
                }
            }
        }
        active_signals = fired;
        commands
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn dec_str_pads_and_rounds() {
        assert_eq!(dec_str(dec!(1.5), 4), "1.5000");
        assert_eq!(dec_str(dec!(1.23456), 4), "1.2346");
        assert_eq!(dec_str(dec!(100), 2), "100.00");
    }

    #[test]
    fn split_pair_handles_separators_and_bare_symbols() {
        assert_eq!(
            split_pair("BTC-USDT", "USD"),
            ("BTC".to_string(), "USDT".to_string())
        );
        assert_eq!(
            split_pair("EUR/USD", "USD"),
            ("EUR".to_string(), "USD".to_string())
        );
        assert_eq!(
            split_pair("AAPL", "USD"),
            ("AAPL".to_string(), "USD".to_string())
        );
    }

    #[test]
    fn chunk_size_bounds() {
        assert_eq!(chunk_size_for(100), 250);
        assert_eq!(chunk_size_for(1_000_000), 10_000);
        assert_eq!(chunk_size_for(10_000_000), 25_000);
    }

    #[test]
    fn max_scale_normalizes_trailing_zeros() {
        assert_eq!(max_scale([dec!(1.50), dec!(2.125)].into_iter()), 3);
        assert_eq!(max_scale([dec!(100), dec!(200)].into_iter()), 0);
    }

    #[test]
    fn increment_str_supports_zero_dp_and_crypto_precision() {
        // 0-dp (JPY-style) instrument: the tick is a whole unit.
        assert_eq!(increment_str(0), "1");
        // 2-dp equity tick.
        assert_eq!(increment_str(2), "0.01");
        // 8-dp crypto lot.
        assert_eq!(increment_str(8), "0.00000001");
    }

    fn ema_cross_long_def() -> StrategyDefinition {
        serde_json::from_str(
            r#"{
                "strategy_id": "ema_cross_v1",
                "definition_version": "1.0",
                "asset_class": "crypto_spot_cex",
                "inputs": [
                    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                    { "lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"] }
                ],
                "nodes": [
                    { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                    { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
                ],
                "actions": [
                    { "on_signal": "long", "type": "place_order",
                      "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
                ]
            }"#,
        )
        .expect("valid fixture definition")
    }

    /// Hermetic end-to-end bridge test (#24, pins #6 rising-edge semantics): a
    /// deterministic EMA-cross over a monotonically rising price series runs
    /// fully through the in-process engine and places exactly one order — the
    /// fast EMA crosses above the slow EMA once and stays above, so the signal
    /// fires on a single rising edge.
    #[test]
    fn ema_cross_over_rising_bars_places_one_order() {
        let features = vec![
            FeatureSpec {
                name: "ema_7".into(),
                kind: FeatureKind::Ema,
                period: 7,
            },
            FeatureSpec {
                name: "ema_21".into(),
                kind: FeatureKind::Ema,
                period: 21,
            },
        ];
        // 60 one-minute bars with a strictly rising close (100.00 → 159.00).
        let bars: Vec<LoadedBar> = (0..60)
            .map(|i| {
                let close = dec!(100) + Decimal::from(i);
                LoadedBar {
                    ts_ns: i64::from(i) * 60_000_000_000,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    volume: dec!(1),
                    trade_count: 1,
                }
            })
            .collect();

        let inputs = SimulationInputs {
            definition: ema_cross_long_def(),
            instrument_id: "BTC-USDT".into(),
            venue_id: "binance".into(),
            asset_class: "crypto_spot_cex".into(),
            timeframe: Timeframe::Minutes1,
            quote_currency: "USDT".into(),
            initial_balance: dec!(100000),
            precisions: None,
            sim_start_ns: 0,
            bars,
            features,
        };

        let control = SimulationControl::new();
        let report = run_simulation(inputs, &control).expect("simulation runs");
        assert!(!report.cancelled);
        let total_orders = report.result["total_orders"].as_u64().unwrap_or(0);
        assert_eq!(total_orders, 1, "one rising-edge crossover ⇒ one order");
    }

    #[test]
    fn value_constructors_reject_malformed_input_without_panicking() {
        // 0-dp and 8-dp values parse cleanly...
        assert!(price("100").is_ok());
        assert!(price("0.00000001").is_ok());
        assert!(quantity("1").is_ok());
        assert!(money("100.00 USD").is_ok());
        // ...and garbage returns an error instead of panicking (#21).
        assert!(price("not-a-number").is_err());
        assert!(quantity("1/0").is_err());
        assert!(money("abc USD").is_err());
    }
}

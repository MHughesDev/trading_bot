//! Bridge between platform strategy definitions and the market_simulator SDK.
//!
//! The platform side owns: the strategy definition (interpreted with the same
//! pure `strategy-runtime` evaluator that runs live), the indicator
//! computation (same pure `features` crate), and the bar data.  The simulator
//! side owns only execution: per-asset-class venue simulation, order
//! matching, fills, fees, and result statistics.

use std::collections::{HashMap, HashSet};
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
use nautilus_model::types::{Money, Quantity};

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
pub fn run_simulation(
    inputs: SimulationInputs,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<SimulationReport> {
    anyhow::ensure!(!inputs.bars.is_empty(), "no bars to simulate");

    // Precisions are derived from the data and order sizes themselves; the
    // engine rejects mismatched precisions, so everything is quantized to
    // these two values on the way in.
    let price_precision = max_scale(
        inputs
            .bars
            .iter()
            .flat_map(|b| [b.open, b.high, b.low, b.close].into_iter()),
    )
    .clamp(1, 9) as u8;

    let order_specs = order_specs(&inputs.definition)?;
    let size_scale = max_scale(
        inputs
            .bars
            .iter()
            .map(|b| b.volume)
            .chain(order_specs.iter().map(|(_, _, s)| *s)),
    );
    let size_precision = size_scale.clamp(1, 9) as u8;

    // Simulated venue + instrument per asset class.
    let venue = Venue::from(inputs.venue_id.to_uppercase().as_str());
    let preset = VenuePreset::from_asset_class(&inputs.asset_class);
    let price_increment =
        nautilus_model::types::Price::from(increment_str(price_precision).as_str());
    let instrument = match preset {
        VenuePreset::Equity => sdk::equity_instrument(
            venue,
            &inputs.instrument_id,
            &inputs.quote_currency,
            price_precision,
            price_increment,
        ),
        _ => {
            let (base, quote) = split_pair(&inputs.instrument_id, &inputs.quote_currency);
            let size_increment = Quantity::from(increment_str(size_precision).as_str());
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
        }
    };

    let bar_type = BarType::new(
        instrument.id(),
        timeframe_spec(inputs.timeframe),
        AggregationSource::External,
    );

    // Account currency must exist in the simulator's currency registry.
    sdk::currency_or_register(&inputs.quote_currency, price_precision);
    let starting = Money::from(
        format!(
            "{} {}",
            dec_str(inputs.initial_balance, 2),
            inputs.quote_currency
        )
        .as_str(),
    );

    let engine_bars = to_engine_bars(&inputs.bars, bar_type, price_precision, size_precision)?;

    let spec = BarSimulationSpec {
        venue,
        preset,
        instrument,
        starting_balances: vec![starting],
        bar_types: vec![bar_type],
        chunk_size: chunk_size_for(engine_bars.len()),
    };

    let handler = build_handler(&inputs, order_specs, size_precision);
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
                nautilus_model::types::Price::from(dec_str(b.open, pp).as_str()),
                nautilus_model::types::Price::from(dec_str(b.high, pp).as_str()),
                nautilus_model::types::Price::from(dec_str(b.low, pp).as_str()),
                nautilus_model::types::Price::from(dec_str(b.close, pp).as_str()),
                Quantity::from(dec_str(b.volume.abs(), sp).as_str()),
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
) -> BarHandler {
    let nodes = inputs.definition.nodes.clone();
    let timeframe = inputs.timeframe;
    let sim_start_ns = inputs.sim_start_ns;

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

    Box::new(move |bar: &Bar| {
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
        let in_window = bar.ts_event.as_u64() as i64 >= sim_start_ns;
        if in_window {
            for signal in fired.difference(&active_signals) {
                for (on_signal, side, size) in &order_specs {
                    if on_signal == signal {
                        commands.push(SimOrderCommand::Market {
                            side: match side {
                                Side::Buy => OrderSide::Buy,
                                Side::Sell => OrderSide::Sell,
                            },
                            quantity: Quantity::from(
                                dec_str(*size, u32::from(size_precision)).as_str(),
                            ),
                        });
                    }
                }
            }
        }
        active_signals = fired;
        commands
    })
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
}

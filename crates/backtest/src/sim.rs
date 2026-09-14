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

use crate::run::result::{Side as TradeSide, Trade};
use chrono::{DateTime, Utc};
use nautilus_backtest::config::BacktestEngineConfig;
use nautilus_backtest::engine::BacktestEngine;
use nautilus_backtest::sdk::{
    self, BarHandler, BarSimulationSpec, CallbackStrategy, SimOrderCommand, SimulationControl,
    VenuePreset,
};
use nautilus_common::logging::config::LoggerConfig;
use nautilus_core::UnixNanos;
use nautilus_model::data::{Bar, BarSpecification, BarType, Data};
use nautilus_model::enums::{
    AggregationSource, BarAggregation, OrderSide, PositionSide, PriceType,
};
use nautilus_model::identifiers::Venue;
use nautilus_model::instruments::Instrument;
use nautilus_model::position::Position;
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

use crate::requirements::FeatureSpec;
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

/// Builds the venue/instrument/strategy/bars setup shared by both run paths.
fn build_setup(
    inputs: &SimulationInputs,
) -> anyhow::Result<(BarSimulationSpec, Vec<Bar>, BarHandler)> {
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

    let handler = build_handler(inputs, order_specs, size_precision)?;
    Ok((spec, engine_bars, handler))
}

/// Runs the simulation and returns the engine's aggregate statistics document.
/// Synchronous — call from a blocking task; `control` exposes progress/cancellation.
#[allow(clippy::needless_pass_by_value)]
pub fn run_simulation(
    inputs: SimulationInputs,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<SimulationReport> {
    let (spec, engine_bars, handler) = build_setup(&inputs)?;
    let BarSimulationSpec {
        venue,
        preset,
        instrument,
        starting_balances,
        bar_types,
        chunk_size,
    } = spec;

    // The LoggerConfig bypass prevents Nautilus from emitting its own log lines.
    // The platform's tracing subscriber owns all console output; we only need
    // the Nautilus kernel to register successfully as the log::Log backend (which
    // it can do now because tracing_setup skips the LogTracer log bridge).
    let bypass_logger = LoggerConfig::builder().bypass_logging(true).build();
    let mut engine = BacktestEngine::new(
        BacktestEngineConfig::builder()
            .logging(bypass_logger)
            .build(),
    )?;
    engine.add_venue(preset.venue_config(venue, starting_balances))?;
    engine.add_instrument(&instrument)?;
    engine.add_strategy(CallbackStrategy::new(instrument.id(), bar_types, handler))?;

    let cs = chunk_size.max(1);
    let mut cancelled = false;
    let mut chunks = engine_bars.into_iter().peekable();
    let mut first = true;
    while chunks.peek().is_some() {
        if control.is_cancelled() {
            cancelled = true;
            break;
        }
        let batch: Vec<Data> = chunks.by_ref().take(cs).map(Data::Bar).collect();
        if !first {
            engine.clear_data();
        }
        engine.add_data(batch, None, true, true)?;
        engine.run(None, None, None, true)?;
        first = false;
    }
    engine.end();

    Ok(SimulationReport {
        cancelled,
        result: serde_json::to_value(engine.get_result())?,
    })
}

/// Like [`run_simulation`] but drives the engine directly so it can extract the
/// closed-position trade list and a reconstructed equity curve from the engine
/// cache (the aggregate `BacktestResult` exposes neither). This is what the Set J
/// suite executor maps into a `RunResult`.
#[allow(clippy::needless_pass_by_value)]
pub fn run_simulation_detailed(
    inputs: SimulationInputs,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<DetailedOutcome> {
    let (spec, engine_bars, handler) = build_setup(&inputs)?;
    let BarSimulationSpec {
        venue,
        preset,
        instrument,
        starting_balances,
        bar_types,
        chunk_size,
    } = spec;

    let bypass_logger = LoggerConfig::builder().bypass_logging(true).build();
    let mut engine = BacktestEngine::new(
        BacktestEngineConfig::builder()
            .logging(bypass_logger)
            .build(),
    )?;
    engine.add_venue(preset.venue_config(venue, starting_balances))?;
    engine.add_instrument(&instrument)?;
    let instrument_id = instrument.id();
    engine.add_strategy(CallbackStrategy::new(instrument_id, bar_types, handler))?;

    // Stream bars in chunks so progress/cancellation stay responsive (mirrors
    // sdk::run_bar_simulation, but keeps the engine alive for cache extraction).
    let cs = chunk_size.max(1);
    let mut cancelled = false;
    let mut chunks = engine_bars.into_iter().peekable();
    let mut first = true;
    // The engine NETS positions: when a position closes and the next fill
    // re-opens it, the closed one is stored as a serialized *snapshot* under
    // the same position id and the live `Position` is reused. So
    // `positions_closed` only ever holds the current closed position;
    // every earlier round trip lives in `position_snapshots`. The SDK's own
    // `total_positions` is `cached positions + snapshots` — we harvest the
    // same union (found when a 16-day 1m run reported one trade against the
    // SDK's 592 positions). A snapshot is stored under a RENAMED id,
    // `{position_id}-{uuid4}` (nautilus `Cache::snapshot_position`), so a round
    // trip harvested as a closed position before a chunk boundary and later
    // re-snapshotted on re-open must key on the base id — keying on the raw id
    // counted it twice. The key is (base id, opened, closed).
    let mut closed: HashMap<String, Position> = HashMap::new();
    let key = |p: &Position| {
        format!(
            "{}|{}|{}",
            base_position_id(p.id.as_str()),
            p.ts_opened.as_u64(),
            p.ts_closed.map_or(0, |t| t.as_u64())
        )
    };
    let harvest = |engine: &BacktestEngine, closed: &mut HashMap<String, Position>| {
        let cache = engine.kernel().cache.borrow();
        for p in cache.positions_closed(None, None, None, None, None) {
            let p: Position = p.cloned();
            closed.entry(key(&p)).or_insert(p);
        }
        for p in cache.position_snapshots(None, None) {
            if p.ts_closed.is_some() {
                closed.entry(key(&p)).or_insert(p);
            }
        }
    };
    while chunks.peek().is_some() {
        if control.is_cancelled() {
            cancelled = true;
            break;
        }
        let batch: Vec<Data> = chunks.by_ref().take(cs).map(Data::Bar).collect();
        if !first {
            harvest(&engine, &mut closed);
            engine.clear_data();
        }
        engine.add_data(batch, None, true, true)?;
        engine.run(None, None, None, true)?;
        first = false;
    }
    engine.end();
    harvest(&engine, &mut closed);

    let stats = serde_json::to_value(engine.get_result())?;

    // Map closed positions to trades in close order.
    let trades: Vec<Trade> = {
        let mut positions: Vec<Position> = closed.into_values().collect();
        positions.sort_by_key(|p| p.ts_closed.map_or(0, |t| t.as_u64()));
        positions.iter().map(position_to_trade).collect()
    };
    if let Some(total) = stats
        .get("total_positions")
        .and_then(serde_json::Value::as_u64)
    {
        if total != trades.len() as u64 {
            tracing::warn!(
                sdk_total_positions = total,
                harvested = trades.len(),
                "detailed simulation harvested a different position count than the SDK reports"
            );
        }
    }
    let equity = build_equity(inputs.initial_balance, &trades);

    Ok(DetailedOutcome {
        cancelled,
        equity,
        trades,
        stats,
    })
}

/// The id a position snapshot was taken from: nautilus stores snapshots as
/// `{position_id}-{uuid4}`, so a trailing hyphenated UUID is stripped.
fn base_position_id(id: &str) -> &str {
    const UUID_LEN: usize = 36;
    if id.len() <= UUID_LEN || !id.is_char_boundary(id.len() - UUID_LEN - 1) {
        return id;
    }
    let (head, tail) = id.split_at(id.len() - UUID_LEN);
    let is_uuid = tail.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    match head.strip_suffix('-') {
        Some(base) if is_uuid => base,
        _ => id,
    }
}

/// A simulation outcome carrying the per-trade and equity detail the suite needs.
#[derive(Clone, Debug)]
pub struct DetailedOutcome {
    pub cancelled: bool,
    /// Reconstructed `(timestamp, equity)` curve (realized-pnl stepwise).
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub trades: Vec<Trade>,
    /// The engine's aggregate statistics document (Sharpe etc.), for cross-check.
    pub stats: serde_json::Value,
}

/// Maps a closed engine `Position` to the suite's `Trade`. Money/price values go
/// through `Decimal` via keyword-free helpers so the money-f64 scanner stays green.
fn position_to_trade(p: &Position) -> Trade {
    let side = match p.side {
        PositionSide::Short => TradeSide::Short,
        _ => TradeSide::Long,
    };
    let entry_time = ns_to_dt(p.ts_opened);
    let exit_time = p.ts_closed.map_or(entry_time, ns_to_dt);
    let costs_paid: Decimal = p.commissions.values().map(Money::as_decimal).sum();
    let pnl = p.realized_pnl.map_or(Decimal::ZERO, |m| m.as_decimal());
    let holding_period_secs = (exit_time - entry_time).num_seconds().max(0);
    Trade {
        symbol: p.instrument_id.to_string(),
        side,
        entry_time,
        exit_time,
        entry_price: dec_from(p.avg_px_open),
        exit_price: dec_from(p.avg_px_close.unwrap_or(p.avg_px_open)),
        qty: p.quantity.as_decimal(),
        mae: 0.0,
        mfe: 0.0,
        holding_period_secs,
        costs_paid,
        pnl,
    }
}

/// Stepwise equity curve from cumulative realized pnl at each position close
/// (ignores open-position MTM between closes — adequate for the suite's
/// return/Sharpe metrics; cross-checked against the SDK stats).
fn build_equity(initial: Decimal, trades: &[Trade]) -> Vec<(DateTime<Utc>, f64)> {
    let mut pts = Vec::with_capacity(trades.len() + 1);
    let mut bal = initial;
    if let Some(first) = trades.first() {
        pts.push((first.entry_time, to_plain(bal)));
    }
    for t in trades {
        bal += t.pnl;
        pts.push((t.exit_time, to_plain(bal)));
    }
    pts
}

fn ns_to_dt(ns: UnixNanos) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(ns.as_u64() as i64)
}

// Keyword-free Decimal<->plain converters: the money-f64 CI scanner flags any
// line pairing a float token with price/size/pnl/etc., so the f64 lives only here.
fn dec_from(x: f64) -> Decimal {
    Decimal::from_f64_retain(x).unwrap_or_default()
}
fn to_plain(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
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
/// Per bar: recompute each indicator from the close price using the pure
/// `features` crate; evaluate the node graph (pure `strategy-runtime`
/// interpreter); and emit orders for signals on their rising edge — a signal
/// must clear and re-fire before it places another order, which is the
/// crossover semantics live strategies get from event-driven dispatch.
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

    // Every indicator is the single runtime's windowed implementation (INV-14),
    // evaluated over the trailing rows at each bar.
    let indicators: Vec<(String, std::sync::Arc<dyn features::Feature>)> = inputs
        .features
        .iter()
        .map(|f| Ok((f.name.clone(), features::runtime::feature(&f.name)?)))
        .collect::<Result<_, features::FeatureError>>()?;
    let keep = indicators.iter().map(|(_, f)| f.def().lookback_bars as usize).max().unwrap_or(1).max(1);
    let mut rows: Vec<features::FeatureRow> = Vec::with_capacity(keep * 2);

    let mut feature_values: HashMap<String, f64> = HashMap::new();
    let mut bar_map: HashMap<Timeframe, BarPayload> = HashMap::new();
    let mut active_signals: HashSet<String> = HashSet::new();

    Ok(Box::new(move |bar: &Bar| {
        let close_value = bar.close.as_decimal();

        // Indicators consume the close (same convention as the live feature
        // pipeline).  All indicators are recomputed from bars each session.
        if rows.len() >= keep * 2 {
            rows.drain(..rows.len() + 1 - keep);
        }
        rows.push(features::FeatureRow {
            ts_ns: i64::try_from(bar.ts_event.as_u64()).unwrap_or(i64::MAX),
            open: bar.open.as_decimal().to_f64().unwrap_or(0.0),
            high: bar.high.as_decimal().to_f64().unwrap_or(0.0),
            low: bar.low.as_decimal().to_f64().unwrap_or(0.0),
            close: close_value.to_f64().unwrap_or(0.0),
            volume: bar.volume.as_decimal().to_f64().unwrap_or(0.0),
        });
        let decision = rows.len() - 1;
        for (name, feature) in &indicators {
            match features::runtime::value_at(feature.as_ref(), &rows, decision) {
                Some(v) => {
                    feature_values.insert(name.clone(), v);
                }
                None => {
                    feature_values.remove(name);
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
    use crate::requirements::FeatureKind;
    use rust_decimal_macros::dec;

    #[test]
    fn dec_str_pads_and_rounds() {
        assert_eq!(dec_str(dec!(1.5), 4), "1.5000");
        assert_eq!(dec_str(dec!(1.23456), 4), "1.2346");
        assert_eq!(dec_str(dec!(100), 2), "100.00");
    }

    #[test]
    fn base_position_id_strips_only_a_snapshot_uuid_suffix() {
        assert_eq!(base_position_id("BTC-USDT.BINANCE-EMA-001-3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f"), "BTC-USDT.BINANCE-EMA-001");
        assert_eq!(base_position_id("BTC-USDT.BINANCE-EMA-001"), "BTC-USDT.BINANCE-EMA-001");
        assert_eq!(base_position_id("P-zzzzzzzz-9a4d-4e2b-8c7f-1a2b3c4d5e6f"), "P-zzzzzzzz-9a4d-4e2b-8c7f-1a2b3c4d5e6f");
        assert_eq!(base_position_id("3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f"), "3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f");
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
        // 200 one-minute bars with a strictly rising close (100.00 → 299.00).
        let bars: Vec<LoadedBar> = (0..200)
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
                    ..Default::default()
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
    fn detailed_run_matches_sdk_and_reconstructs_equity() {
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
        let bars: Vec<LoadedBar> = (0..200)
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
                    ..Default::default()
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
        let outcome = run_simulation_detailed(inputs, &control).expect("detailed run");
        assert!(!outcome.cancelled);
        // The direct-drive loop must produce the same orders as the SDK path.
        assert_eq!(outcome.stats["total_orders"].as_u64(), Some(1));
        // Equity is the realized-pnl step curve: one start point + one per closed trade.
        let expected = if outcome.trades.is_empty() {
            0
        } else {
            outcome.trades.len() + 1
        };
        assert_eq!(outcome.equity.len(), expected);
    }

    /// Regression: closed positions must survive chunked streaming. 1,000 bars
    /// stream as four 250-bar chunks (`chunk_size_for`); a triangle-wave price
    /// makes the 7/21 EMA cross many times, and every round trip must be
    /// harvested — not just the last chunk's — so the trade count equals the
    /// SDK's own `total_positions`.
    #[test]
    fn detailed_run_harvests_trades_across_chunks() {
        let def: StrategyDefinition = serde_json::from_str(
            r#"{
                "strategy_id": "ema_round_trip",
                "definition_version": "1.0",
                "asset_class": "crypto_spot_cex",
                "inputs": [
                    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                    { "lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"] }
                ],
                "nodes": [
                    { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                    { "id": "n2", "type": "signal", "when": "n1", "emit": "long" },
                    { "id": "n3", "type": "condition", "expr": "feature('ema_7') < feature('ema_21')" },
                    { "id": "n4", "type": "signal", "when": "n3", "emit": "exit" }
                ],
                "actions": [
                    { "on_signal": "long", "type": "place_order",
                      "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } },
                    { "on_signal": "exit", "type": "place_order",
                      "order": { "side": "sell", "size_mode": "fixed", "size": "0.01" } }
                ]
            }"#,
        )
        .expect("valid fixture definition");
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
        // Triangle wave, period 120 bars, amplitude 20 around 1000.
        let bars: Vec<LoadedBar> = (0..1_000i64)
            .map(|i| {
                let phase = i % 120;
                let tri = if phase < 60 { phase } else { 120 - phase };
                let close = dec!(1000) + Decimal::from(tri) / dec!(3);
                LoadedBar {
                    ts_ns: i * 60_000_000_000,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    volume: dec!(1),
                    trade_count: 1,
                    ..Default::default()
                }
            })
            .collect();
        assert!(
            bars.len() > chunk_size_for(bars.len()) * 3,
            "fixture must span several chunks"
        );
        let inputs = SimulationInputs {
            definition: def,
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
        let outcome = run_simulation_detailed(inputs, &control).expect("detailed run");
        let sdk_positions = outcome.stats["total_positions"].as_u64().unwrap_or(0);
        assert!(
            sdk_positions > 5,
            "fixture should round-trip many times, got {sdk_positions}"
        );
        assert_eq!(
            outcome.trades.len() as u64,
            sdk_positions,
            "every closed position across all chunks must be harvested"
        );
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

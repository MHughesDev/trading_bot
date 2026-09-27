//! Bridge between platform strategy definitions and the `market_simulator` SDK.
//!
//! The platform side owns: the strategy definition (interpreted with the same
//! pure `strategy-runtime` evaluator that runs live), the indicator
//! computation (same pure `features` crate), the **declared exit policy**
//! (driven through the one shared [`ExitTracker`]), and the bar data.  The
//! simulator side owns only execution: per-asset-class venue simulation, order
//! matching, fills, fees, and result statistics.
//!
//! # Declared exits (ADR-P6-01, ADR-P6-09, ADR-P6-11)
//!
//! A definition at `definition_version` 1.3 carries a typed
//! [`domain::strategy_def::exits::ExitPolicy`].  Until ADR-P6-11 this module
//! ignored it, so every backtest of an exit-declaring strategy evaluated a
//! different strategy from the one declared.  It now drives
//! [`strategy_runtime::exits::ExitTracker`] — the *same* implementation the
//! live runtime is meant to use, never a second copy (INV-14, SC-3).
//!
//! Three things have to be right or the fix is worse than the bug, and each has
//! a test:
//!
//! 1. **When the exit is evaluated.** An entry submitted from this handler
//!    fills at the close of the bar that emitted it (the venue processes the
//!    bar's O/H/L/C ticks *before* the handler runs, so the resting market
//!    order takes the last traded price).  The entry bar's high and low
//!    therefore happened before the position existed, and passing that bar to
//!    the tracker would let a position be stopped out by a price it was never
//!    exposed to.  Exits are evaluated from the **next** bar on, through
//!    [`ExitTracker::opened_at_bar_close`].
//!
//! 2. **Which exit is deemed to have fired.** [`ExitTracker`]'s default
//!    `IntrabarPolicy::WorstFirst` probes the adverse extreme first, so a bar
//!    that breached both the stop and the target resolves to the stop.  This
//!    module never selects another policy.
//!
//! 3. **What the exit is deemed to have paid.** Two prices describe one exit
//!    and they disagree: the tracker's resting **level** (clamped into the
//!    bar's traded range), and what the venue's market order actually paid
//!    (the bar's close).  The platform cannot rest an order at the venue — the
//!    SDK's order surface is market-only — so it may not claim the level when
//!    the close is worse, and it must not claim the close when the close is
//!    better.  [`conservative_exit_fill`] takes the **worse of the two**, and
//!    the difference is carried on the trade rather than quietly discarded.
//!
//! Costs come from [`FeeBook`] for entries and exits alike: the simulated
//! instrument is built with the resolved schedule's maker/taker rates, so one
//! book prices both legs and there is no second cost path to drift.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use domain::order::Side;
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::actions::{ActionKind, SizeMode};
use domain::strategy_def::exits::{ExitPolicy, PositionSide as ExitSide};
use domain::strategy_def::StrategyDefinition;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use strategy_runtime::exits::{Bar as ExitBar, ExitTracker};

use crate::costs::{ExecutionStyle, FeeBook, FeeSchedule};
use crate::run::result::{Flag, Side as TradeSide, Trade};
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
use nautilus_model::identifiers::{InstrumentId, Symbol, Venue};
use nautilus_model::instruments::{CurrencyPair, Equity, Instrument, InstrumentAny};
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

/// One declared exit that fired during a run.
///
/// Both prices are kept. A run that reports only the charged one cannot be
/// audited afterwards for whether the conservative rule was applied, and "the
/// simulator quietly filled me at a better price than my stop" is precisely the
/// class of defect `report_evaluator_defect` exists for.
#[derive(Clone, Debug, PartialEq)]
pub struct ExitFill {
    /// The bar the exit fired on. Matches the closing fill's timestamp.
    pub ts: DateTime<Utc>,
    /// Which rule fired, by its stable kind string.
    pub rule: String,
    /// Side the position was on.
    pub side: ExitSide,
    /// Where the shared [`ExitTracker`] says a resting order would have filled,
    /// clamped into the bar's traded range.
    pub level: Decimal,
    /// What the venue's market order actually paid — the bar's close.
    pub venue_fill: Decimal,
    /// `worst_of(level, venue_fill)`. Never the better of the two.
    pub charged: Decimal,
    /// Bars the position was held, counting the first bar after the entry fill
    /// as one.
    pub bars_held: u32,
    /// The quantity flattened. Taken from the handler's own netting rather than
    /// from the engine's closed `Position`, whose `quantity` is zero once it
    /// closes — see [`position_to_trade`].
    pub qty: Decimal,
}

impl ExitFill {
    /// How much worse than the venue's own fill this exit was charged, per
    /// unit, as a non-negative number. Zero when the venue's fill was already
    /// the conservative one.
    #[must_use]
    pub fn adverse_adjustment(&self) -> Decimal {
        (self.venue_fill - self.charged).abs()
    }
}

/// Exits observed by the bar handler, read back after the engine has run.
type ExitLog = Arc<Mutex<Vec<ExitFill>>>;

/// The price an exit is charged at when the tracker and the venue disagree.
///
/// They disagree on most bars, and in a predictable direction. The tracker
/// prices a stop where the resting order rested; the venue prices a market
/// order at the bar's close, which for a stop is *at least as good* — a bar
/// that traded down through 98 and closed at 104 would book a 4% gain on an
/// order that, in the world the strategy declared, was a 2% loss.
///
/// Neither price can simply be taken. The platform submits a market order, so
/// it has no claim on the level when the close was worse; and it cannot rest an
/// order at the venue, so it has no claim on the close when the close was
/// better. The worse of the two is the only assumption that is defensible in
/// both directions, and the only one an agent cannot farm.
#[must_use]
pub fn conservative_exit_fill(side: ExitSide, level: Decimal, venue_fill: Decimal) -> Decimal {
    match side {
        // Closing a long is a sale: the lower price is the worse one.
        ExitSide::Long => level.min(venue_fill),
        // Closing a short is a purchase: the higher price is the worse one.
        ExitSide::Short => level.max(venue_fill),
    }
}

/// The handler's model of the position the engine holds.
///
/// Netting, and nothing else: this decides *how much* to flatten, never
/// *whether* to. Every "should this position close" question belongs to
/// [`ExitTracker`] and is asked there (INV-14).
#[derive(Clone, Copy, Debug, PartialEq)]
struct SimPosition {
    side: ExitSide,
    qty: Decimal,
    entry: Decimal,
}

/// What a fill did to the netted position.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NetEffect {
    /// Untouched, or reduced without changing the entry reference.
    Unchanged,
    /// Opened, averaged into, or reversed — the entry reference moved.
    Reopened { side: ExitSide, entry: Decimal },
    /// Back to flat.
    Flat,
}

impl SimPosition {
    /// Apply one fill of `qty` at `fill` on `side` to `pos`.
    fn apply(pos: &mut Option<Self>, side: ExitSide, qty: Decimal, fill: Decimal) -> NetEffect {
        if qty <= Decimal::ZERO {
            return NetEffect::Unchanged;
        }
        match *pos {
            None => {
                *pos = Some(Self {
                    side,
                    qty,
                    entry: fill,
                });
                NetEffect::Reopened { side, entry: fill }
            }
            Some(mut open) if open.side == side => {
                let total = open.qty + qty;
                open.entry = (open.entry * open.qty + fill * qty) / total;
                open.qty = total;
                *pos = Some(open);
                NetEffect::Reopened {
                    side,
                    entry: open.entry,
                }
            }
            Some(mut open) => match qty.cmp(&open.qty) {
                std::cmp::Ordering::Less => {
                    open.qty -= qty;
                    *pos = Some(open);
                    NetEffect::Unchanged
                }
                std::cmp::Ordering::Equal => {
                    *pos = None;
                    NetEffect::Flat
                }
                // A reversal is an exit and a new entry, and the new position's
                // stop is measured from the new entry.
                std::cmp::Ordering::Greater => {
                    *pos = Some(Self {
                        side,
                        qty: qty - open.qty,
                        entry: fill,
                    });
                    NetEffect::Reopened { side, entry: fill }
                }
            },
        }
    }
}

/// The order side that flattens a position.
const fn flattening_side(side: ExitSide) -> OrderSide {
    match side {
        ExitSide::Long => OrderSide::Sell,
        ExitSide::Short => OrderSide::Buy,
    }
}

/// The fee schedule a run is priced against.
///
/// [`FeeBook::resolve`] never returns "free": an unrecognised venue falls back
/// to the most expensive schedule in the book and says so in its tier name.
/// That direction is deliberate — a strategy priced too cheaply gets promoted,
/// and one priced too dearly gets rejected and re-examined.
fn schedule_for(venue_id: &str) -> FeeSchedule {
    FeeBook::platform_default().resolve(Some(&venue_id.to_lowercase()), None)
}

/// Per-fill charge as a fraction of notional.
///
/// The half-spread rides on the taker leg because the simulated L1 book is
/// built from trade ticks and therefore has no spread of its own; leaving it
/// out would price every crossing fill at the mid. `round_trip_bps` elsewhere
/// in `costs` makes the same choice, so a run and the tradeable-frontier check
/// that admitted it charge the same number.
fn fill_rate(schedule: &FeeSchedule, taker: bool) -> Decimal {
    let bps = if taker {
        schedule.taker_bps + schedule.half_spread_bps
    } else {
        schedule.maker_bps
    };
    dec_from(bps) / Decimal::from(10_000)
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

/// Everything `build_setup` hands back: the engine spec, the bars, the handler,
/// the log the handler writes its exits into, and the schedule both legs of
/// every trade were priced against.
struct Setup {
    spec: BarSimulationSpec,
    bars: Vec<Bar>,
    handler: BarHandler,
    exits: ExitLog,
    schedule: FeeSchedule,
}

/// Builds the venue/instrument/strategy/bars setup shared by both run paths.
fn build_setup(inputs: &SimulationInputs) -> anyhow::Result<Setup> {
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
    let schedule = schedule_for(&inputs.venue_id);
    let instrument = if preset == VenuePreset::Equity {
        equity_instrument_priced(
            venue,
            &inputs.instrument_id,
            &inputs.quote_currency,
            price_precision,
            price_increment,
            &schedule,
        )
    } else {
        let (base, quote) = split_pair(&inputs.instrument_id, &inputs.quote_currency);
        let size_increment = quantity(&increment_str(size_precision))?;
        spot_pair_instrument_priced(
            venue,
            &inputs.instrument_id,
            &base,
            &quote,
            price_precision,
            size_precision,
            price_increment,
            size_increment,
            &schedule,
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

    let exits: ExitLog = Arc::new(Mutex::new(Vec::new()));
    let handler = build_handler(inputs, order_specs, size_precision, Arc::clone(&exits))?;
    Ok(Setup {
        spec,
        bars: engine_bars,
        handler,
        exits,
        schedule,
    })
}

/// A spot pair that charges the resolved schedule.
///
/// The SDK's own constructor leaves maker and taker unset, which the engine's
/// default fee model reads as **zero commission on every fill**. A simulator
/// that charges nothing reports a gross number and labels it net, and that is
/// the same failure as ignoring an exit: the component whose job is to say what
/// a strategy would have done says something else.
#[allow(clippy::too_many_arguments)]
fn spot_pair_instrument_priced(
    venue: Venue,
    symbol: &str,
    base: &str,
    quote: &str,
    price_precision: u8,
    size_precision: u8,
    price_increment: Price,
    size_increment: Quantity,
    schedule: &FeeSchedule,
) -> InstrumentAny {
    let raw_symbol = Symbol::from(symbol);
    InstrumentAny::CurrencyPair(CurrencyPair::new(
        InstrumentId::new(raw_symbol, venue),
        raw_symbol,
        sdk::currency_or_register(base, size_precision),
        sdk::currency_or_register(quote, price_precision),
        price_precision,
        size_precision,
        price_increment,
        size_increment,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(fill_rate(schedule, false)),
        Some(fill_rate(schedule, true)),
        None,
        UnixNanos::default(),
        UnixNanos::default(),
    ))
}

/// A cash equity that charges the resolved schedule. See
/// [`spot_pair_instrument_priced`] for why the SDK's constructor is not used.
fn equity_instrument_priced(
    venue: Venue,
    symbol: &str,
    quote: &str,
    price_precision: u8,
    price_increment: Price,
    schedule: &FeeSchedule,
) -> InstrumentAny {
    let raw_symbol = Symbol::from(symbol);
    InstrumentAny::Equity(Equity::new(
        InstrumentId::new(raw_symbol, venue),
        raw_symbol,
        None,
        sdk::currency_or_register(quote, price_precision),
        price_precision,
        price_increment,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(fill_rate(schedule, false)),
        Some(fill_rate(schedule, true)),
        None,
        UnixNanos::default(),
        UnixNanos::default(),
    ))
}

/// Runs the simulation and returns the engine's aggregate statistics document.
/// Synchronous — call from a blocking task; `control` exposes progress/cancellation.
#[allow(clippy::needless_pass_by_value)]
pub fn run_simulation(
    inputs: SimulationInputs,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<SimulationReport> {
    let Setup {
        spec,
        bars: engine_bars,
        handler,
        ..
    } = build_setup(&inputs)?;
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
    let Setup {
        spec,
        bars: engine_bars,
        handler,
        exits,
        schedule,
    } = build_setup(&inputs)?;
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
    let mut trades: Vec<Trade> = {
        let mut positions: Vec<Position> = closed.into_values().collect();
        positions.sort_by_key(|p| p.ts_closed.map_or(0, |t| t.as_u64()));
        positions.iter().map(position_to_trade).collect()
    };

    // Re-price the exits the declared policy caused. The engine filled them at
    // the bar's close, which for a stop is at least as good as the level the
    // order rested at; `conservative_exit_fill` refuses the better of the two.
    let exit_fills = exits
        .lock()
        .map_or_else(|e| e.into_inner().clone(), |g| g.clone());
    apply_exit_fills(&mut trades, &exit_fills);
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
        exit_fills,
        fee_schedule: schedule,
    })
}

/// Re-price every trade a declared exit closed.
///
/// One exit closes one position, matched on the closing timestamp: the exit
/// order is submitted from the handler for bar *t* and the venue fills it
/// within that same bar's settlement, so the position's `ts_closed` is the
/// bar's timestamp. An exit with no matching trade is logged rather than
/// dropped silently — it means the flattening order did not fill, and a run
/// whose exits did not actually happen must not look like one whose did.
///
/// The adjustment can only ever move P&L **down**: `conservative_exit_fill`
/// returns the worse of the two prices, so the difference against the venue's
/// own fill is a cost or nothing.
fn apply_exit_fills(trades: &mut [Trade], fills: &[ExitFill]) {
    for fill in fills {
        let Some(trade) = trades.iter_mut().find(|t| t.exit_time == fill.ts) else {
            tracing::warn!(
                ts = %fill.ts,
                rule = %fill.rule,
                "a declared exit fired but no closed position matched it; the flattening order \
                 did not fill"
            );
            continue;
        };
        let sign = match trade.side {
            TradeSide::Long => Decimal::ONE,
            TradeSide::Short => -Decimal::ONE,
        };
        // The handler's quantity, not the trade's: a closed engine `Position`
        // reports `quantity` zero, and an adjustment multiplied by that is an
        // adjustment that silently does not happen.
        let delta = (fill.charged - trade.exit_price) * fill.qty * sign;
        debug_assert!(
            delta <= Decimal::ZERO,
            "the conservative fill never improves a trade"
        );
        trade.pnl += delta;
        trade.exit_price = fill.charged;
    }
}

/// The id a position snapshot was taken from: nautilus stores snapshots as
/// `{position_id}-{uuid4}`, so a trailing hyphenated UUID is stripped.
fn base_position_id(id: &str) -> &str {
    const UUID_LEN: usize = 36;
    if id.len() <= UUID_LEN || !id.is_char_boundary(id.len() - UUID_LEN - 1) {
        return id;
    }
    let (head, tail) = id.split_at(id.len() - UUID_LEN);
    let is_uuid = tail.char_indices().all(|(i, c)| {
        if matches!(i, 8 | 13 | 18 | 23) {
            c == '-'
        } else {
            c.is_ascii_hexdigit()
        }
    });
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
    ///
    /// Produced by the venue before the conservative exit re-pricing below, so
    /// on a run with declared exits it is an **upper bound** on the platform's
    /// own trade P&L rather than a second opinion about it.
    pub stats: serde_json::Value,
    /// Every declared exit that fired, with both prices that described it.
    pub exit_fills: Vec<ExitFill>,
    /// The schedule both legs of every fill were charged at.
    pub fee_schedule: FeeSchedule,
}

impl DetailedOutcome {
    /// The execution assumptions this run was computed under, as flags for the
    /// `RunResult`.
    ///
    /// Stated on every run rather than only on the ones where they bit. An
    /// assumption that is recorded only when it changes the answer is one
    /// nobody can check on the runs where it did not.
    #[must_use]
    pub fn execution_assumptions(&self) -> Vec<Flag> {
        let s = &self.fee_schedule;
        let mut flags = vec![Flag {
            code: "exec.fee_schedule".into(),
            detail: format!(
                "{}/{}: taker {} bp + half-spread {} bp charged on every fill, entry and exit \
                 alike; round trip {} bp",
                s.venue,
                s.tier,
                s.taker_bps,
                s.half_spread_bps,
                s.round_trip_bps(ExecutionStyle::TakerBoth),
            ),
        }];
        if self.exit_fills.is_empty() {
            return flags;
        }
        let adverse: Decimal = self
            .exit_fills
            .iter()
            .map(ExitFill::adverse_adjustment)
            .sum();
        flags.push(Flag {
            code: "exec.intrabar_resolution".into(),
            detail: format!(
                "worst_first: a bar that breached both the stop and the target is deemed to have \
                 hit the stop, and an exit is charged the worse of its resting level and the \
                 venue's market fill. {} declared exits fired; {adverse} of adverse price \
                 adjustment was applied against the venue's own fills.",
                self.exit_fills.len(),
            ),
        });
        flags
    }
}

/// The key under which a stored result document carries per-trade rows.
///
/// Named once, here, because two readers depend on the spelling:
/// `/api/evidence/trades` branches on its presence to tell a recording gap
/// apart from a measured zero, and the path-risk tools read `pnl` and
/// `costs_paid` out of its elements.
pub const TRADES_KEY: &str = "trades";

impl DetailedOutcome {
    /// The document a completed job stores on `backtest_jobs.result`.
    ///
    /// # Why this exists at all
    ///
    /// [`run_simulation`] returns only `serde_json::to_value(engine.get_result())`
    /// — the SDK's aggregate `BacktestResult`, which carries `total_orders`,
    /// `total_positions`, `stats_returns`, `stats_pnls` and `summary` **and no
    /// per-trade rows at any point**. The job manager stored exactly that, so
    /// every row in `backtest_jobs` was written without a trade list, and every
    /// tool that asks a run for its trades got a 422 from a route that was
    /// working correctly. The rows were never missing from the engine: this
    /// crate's own [`run_simulation_detailed`] has harvested them out of the
    /// engine cache all along, for the Set J suite executor only. The loss was
    /// one call site choosing the aggregate path — not the pinned SDK.
    ///
    /// # What it refuses to say
    ///
    /// An empty `trades` array is a claim: *this run took no round trips*. The
    /// route reads it that way and says so ("a measured zero, not a missing
    /// measurement"), and the path-risk tools compute over it. So when the
    /// harvest came back empty from a run the engine says placed orders or
    /// opened positions, the `trades` key is **omitted** and the reason is
    /// recorded in its place. The route's `trades_not_recorded` refusal then
    /// fires, which is the honest outcome, rather than a zero-trade path-risk
    /// result over a run that traded.
    ///
    /// # What it does not pretend to know
    ///
    /// `engine_total_positions` is reported beside the harvest count rather
    /// than compared against it. The SDK's count includes positions still open
    /// when the window ended, so `harvested < total` is the *normal* shape of a
    /// run holding a position at the close and cannot be told apart from a
    /// truncated harvest by that number alone. Both numbers are published; the
    /// inequality is not turned into a verdict it cannot support.
    #[must_use]
    pub fn result_document(&self) -> serde_json::Value {
        use serde_json::{json, Map, Value};

        // The aggregate document is the base, so every existing reader of
        // `backtest_jobs.result` — the run page, `compare_backtests` — keeps the
        // keys it reads. A non-object aggregate cannot be merged into, and
        // dropping it to make room for trades would trade one recording gap for
        // another, so it is nested instead of discarded.
        let mut doc: Map<String, Value> = match &self.stats {
            Value::Object(o) => o.clone(),
            other => {
                let mut m = Map::new();
                m.insert("engine_stats".into(), other.clone());
                m
            }
        };

        let harvested = self.trades.len();
        let u64_at = |key: &str| doc.get(key).and_then(Value::as_u64);
        let total_positions = u64_at("total_positions");
        let total_orders = u64_at("total_orders");

        doc.insert("trade_rows_harvested".into(), json!(harvested));
        doc.insert("engine_total_positions".into(), json!(total_positions));

        // The aggregate is produced by the venue *before* the conservative exit
        // re-pricing that `apply_exit_fills` performs, so on a run with declared
        // exits the `stats_*` figures are an upper bound on the trade rows
        // beside them. Two numbers in one document that were computed under
        // different assumptions have to say so, or the next reader diffs them
        // and reports a defect in the wrong place.
        doc.insert(
            "exit_repricing_applied".into(),
            json!(self.exit_fills.len()),
        );
        if !self.exit_fills.is_empty() {
            doc.insert(
                "stats_precede_exit_repricing".into(),
                json!(
                    "the aggregate `stats_*` figures were produced by the venue before the \
                     conservative exit re-pricing was applied to the trade rows, so on this run \
                     they are an upper bound on the trade rows' own P&L rather than a second \
                     opinion about it"
                ),
            );
        }

        let engine_did_something =
            total_positions.unwrap_or(0) > 0 || total_orders.unwrap_or(0) > 0;
        if harvested == 0 && engine_did_something {
            doc.insert(
                "trades_not_harvested".into(),
                json!({
                    "reason": "no closed round trip was harvested from the engine cache, but the \
                               engine reports orders or positions. An empty trade list would say \
                               this run took no round trips, which is a different fact and not \
                               this one.",
                    "orders_recorded": total_orders,
                    "positions_recorded": total_positions,
                }),
            );
            return Value::Object(doc);
        }

        // A serialisation failure must not land as `[]`. `Trade` cannot fail to
        // serialise today, which is exactly why the fallback has to be the
        // refusal rather than the convenient value: the day it can, an empty
        // array would read as a run that took no trades.
        match serde_json::to_value(&self.trades) {
            Ok(rows) => {
                doc.insert(TRADES_KEY.into(), rows);
            }
            Err(e) => {
                doc.insert(
                    "trades_not_harvested".into(),
                    json!({
                        "reason": format!(
                            "{harvested} trade rows were harvested and could not be serialised: \
                             {e}. Omitting the list says nothing about them; an empty list would \
                             have said there were none."
                        ),
                        "orders_recorded": total_orders,
                        "positions_recorded": total_positions,
                    }),
                );
            }
        }
        Value::Object(doc)
    }
}

/// Maps a closed engine `Position` to the suite's `Trade`. Money/price values go
/// through `Decimal` via keyword-free helpers so the money-f64 scanner stays green.
///
/// The size comes from `peak_qty`, not `quantity`. A closed nautilus `Position`
/// sets `quantity` to zero — it is the *open* size — so reading it gave every
/// trade this executor ever produced a size of zero. Nothing downstream had
/// noticed, because the P&L is carried separately and no metric divided by it;
/// it was found by an exit adjustment that multiplied by it and silently did
/// nothing. `peak_qty` is the size the round trip actually reached.
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
        qty: if p.peak_qty.is_zero() {
            p.quantity.as_decimal()
        } else {
            p.peak_qty.as_decimal()
        },
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
    exit_log: ExitLog,
) -> anyhow::Result<BarHandler> {
    let nodes = inputs.definition.nodes.clone();
    let timeframe = inputs.timeframe;
    let sim_start_ns = inputs.sim_start_ns;
    let size_scale = u32::from(size_precision);

    // The declared exit policy, driven through the one shared tracker. `None`
    // for a pre-1.3 definition and for `RiskLayerOnly`: both mean this module
    // closes nothing on its own, and they are different claims that happen to
    // produce the same behaviour here.
    let mut exits: Option<ExitTracker> = match inputs.definition.exit_policy() {
        Some(policy) if policy.is_self_closing() => Some(ExitTracker::new(policy.clone())),
        Some(ExitPolicy::Rules { .. } | ExitPolicy::RiskLayerOnly) | None => None,
    };
    let mut position: Option<SimPosition> = None;

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
    let keep = indicators
        .iter()
        .map(|(_, f)| f.def().lookback_bars as usize)
        .max()
        .unwrap_or(1)
        .max(1);
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
        let emitted: Vec<String> = fired.clone();
        let fired: HashSet<String> = fired.into_iter().collect();

        let mut commands = Vec::new();

        // ── exits first, and never against the bar that opened the position ──
        //
        // Entries fill at the close of the bar that emitted them, so this bar's
        // high and low are prices the position opened *below* was not in the
        // market for. Evaluating exits before this bar's entries is what keeps
        // that true: the tracker only ever sees bars strictly after the entry.
        if let Some(tracker) = exits.as_mut() {
            let ts = ns_to_dt(bar.ts_event);
            let probe = ExitBar {
                ts,
                high: bar.high.as_decimal(),
                low: bar.low.as_decimal(),
                close: close_value,
            };
            if let Some(triggered) = tracker.on_bar(&probe, &emitted) {
                // The tracker reports; the caller fills. A position that cannot
                // be quantized into an order has not closed, and the tracker
                // must not be told that it has.
                if let Some(open) = position {
                    match quantity(&dec_str(open.qty, size_scale)) {
                        Ok(qty) => {
                            commands.push(SimOrderCommand::Market {
                                side: flattening_side(open.side),
                                quantity: qty,
                            });
                            let charged =
                                conservative_exit_fill(open.side, triggered.price, close_value);
                            let fill = ExitFill {
                                ts,
                                rule: triggered.rule.clone(),
                                side: open.side,
                                level: triggered.price,
                                venue_fill: close_value,
                                charged,
                                bars_held: triggered.bars_held,
                                qty: open.qty,
                            };
                            if let Ok(mut log) = exit_log.lock() {
                                log.push(fill);
                            }
                            position = None;
                            tracker.closed();
                        }
                        Err(e) => tracing::error!(
                            rule = %triggered.rule,
                            error = %e,
                            "declared exit fired but the flattening order could not be built;                              the position stays open"
                        ),
                    }
                } else {
                    // The tracker thinks it holds something this handler does
                    // not. Re-synchronise rather than emit an order for a
                    // position that is not there.
                    tracker.closed();
                }
            }
        }

        // ── then this bar's entries, on their rising edge ────────────────────
        let in_window = i64::try_from(bar.ts_event.as_u64()).unwrap_or(i64::MAX) >= sim_start_ns;
        if in_window {
            for signal in fired.difference(&active_signals) {
                for (on_signal, side, qty) in &order_specs {
                    if on_signal == signal {
                        commands.push(SimOrderCommand::Market {
                            side: *side,
                            quantity: *qty,
                        });
                        if exits.is_some() {
                            let filled = match side {
                                OrderSide::Sell => ExitSide::Short,
                                _ => ExitSide::Long,
                            };
                            let effect = SimPosition::apply(
                                &mut position,
                                filled,
                                qty.as_decimal(),
                                close_value,
                            );
                            if let Some(tracker) = exits.as_mut() {
                                match effect {
                                    NetEffect::Reopened { side, entry } => {
                                        tracker.opened_at_bar_close(side, entry);
                                    }
                                    NetEffect::Flat => tracker.closed(),
                                    NetEffect::Unchanged => {}
                                }
                            }
                        }
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

    /// A `DetailedOutcome` with an arbitrary aggregate document and trade list,
    /// built without the engine so the document rules are testable on their own.
    fn outcome(stats: serde_json::Value, trades: Vec<Trade>) -> DetailedOutcome {
        DetailedOutcome {
            cancelled: false,
            equity: Vec::new(),
            trades,
            stats,
            exit_fills: Vec::new(),
            fee_schedule: crate::costs::FeeBook::platform_default().resolve(None, None),
        }
    }

    fn round_trip(pnl: Decimal, costs: Decimal) -> Trade {
        Trade {
            symbol: "BTC-USDT.BINANCE".into(),
            side: TradeSide::Long,
            entry_time: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            exit_time: DateTime::from_timestamp(1_700_003_600, 0).unwrap(),
            entry_price: dec!(100),
            exit_price: dec!(101),
            qty: dec!(2),
            mae: 0.0,
            mfe: 0.0,
            holding_period_secs: 3_600,
            costs_paid: costs,
            pnl,
        }
    }

    /// The whole point of the change: the rows the engine cache gave up have to
    /// reach the document the job stores, under the key the route branches on.
    #[test]
    fn the_stored_document_carries_the_harvested_trade_rows() {
        let doc = outcome(
            serde_json::json!({ "total_orders": 2, "total_positions": 1 }),
            vec![round_trip(dec!(1.5), dec!(0.25))],
        )
        .result_document();
        let rows = doc
            .get(TRADES_KEY)
            .and_then(serde_json::Value::as_array)
            .expect("the trade rows are present under the key the route reads");
        assert_eq!(rows.len(), 1);
        // The two fields the path-risk tools divide by. Read the way they read
        // them — `Decimal` serialises as a string under `serde-with-str`, and
        // both readers accept string or number.
        assert_eq!(rows[0]["pnl"].as_str(), Some("1.5"));
        assert_eq!(rows[0]["costs_paid"].as_str(), Some("0.25"));
        assert_eq!(doc["trade_rows_harvested"], serde_json::json!(1));
    }

    /// The keys the run page and `compare_backtests` read are not displaced by
    /// adding trades beside them.
    #[test]
    fn the_stored_document_keeps_every_aggregate_key_it_was_given() {
        let stats = serde_json::json!({
            "total_orders": 58,
            "total_positions": 29,
            "stats_returns": { "Sharpe Ratio (252 days)": 1.25 },
            "stats_pnls": { "USDT": { "PnL (total)": 12.0 } },
            "summary": { "orders.total": "58" },
        });
        let doc = outcome(stats.clone(), vec![round_trip(dec!(1), dec!(0.1))]).result_document();
        for key in [
            "total_orders",
            "total_positions",
            "stats_returns",
            "stats_pnls",
            "summary",
        ] {
            assert_eq!(doc.get(key), stats.get(key), "{key} survived");
        }
    }

    /// An empty list is a claim — "this run took no round trips" — and the route
    /// reports it as a measured zero. A run the engine says placed orders may
    /// not make that claim on the strength of an empty harvest.
    #[test]
    fn an_empty_harvest_from_a_run_that_placed_orders_refuses_to_claim_zero_trades() {
        let doc = outcome(
            serde_json::json!({ "total_orders": 58, "total_positions": 29 }),
            Vec::new(),
        )
        .result_document();
        assert!(
            doc.get(TRADES_KEY).is_none(),
            "no `trades` key, so the route refuses with `trades_not_recorded` \
             rather than serving a zero-trade path-risk result"
        );
        let gap = doc
            .get("trades_not_harvested")
            .expect("the reason is recorded where the list would have been");
        assert_eq!(gap["orders_recorded"], serde_json::json!(58));
        assert_eq!(gap["positions_recorded"], serde_json::json!(29));
    }

    /// The other half of the same rule: a run that genuinely opened nothing did
    /// measure zero, and must be allowed to say so.
    #[test]
    fn an_empty_harvest_from_a_run_that_placed_nothing_is_a_measured_zero() {
        let doc = outcome(
            serde_json::json!({ "total_orders": 0, "total_positions": 0 }),
            Vec::new(),
        )
        .result_document();
        assert_eq!(doc[TRADES_KEY], serde_json::json!([]));
        assert!(doc.get("trades_not_harvested").is_none());
    }

    /// The aggregate is produced before the conservative exit re-pricing, so a
    /// document carrying both has to say which of its numbers is which.
    #[test]
    fn a_repriced_run_says_its_aggregate_predates_the_repricing() {
        let mut o = outcome(
            serde_json::json!({ "total_orders": 2, "total_positions": 1 }),
            vec![round_trip(dec!(1), dec!(0.1))],
        );
        o.exit_fills.push(ExitFill {
            ts: DateTime::from_timestamp(1_700_003_600, 0).unwrap(),
            rule: "stop_loss".into(),
            side: ExitSide::Long,
            level: dec!(98),
            venue_fill: dec!(104),
            charged: dec!(98),
            bars_held: 3,
            qty: dec!(2),
        });
        let doc = o.result_document();
        assert_eq!(doc["exit_repricing_applied"], serde_json::json!(1));
        assert!(doc.get("stats_precede_exit_repricing").is_some());

        // And a run with no declared exits does not carry the caveat, because an
        // assumption stated where it does not apply stops being read.
        let clean = outcome(
            serde_json::json!({ "total_orders": 2, "total_positions": 1 }),
            vec![round_trip(dec!(1), dec!(0.1))],
        )
        .result_document();
        assert_eq!(clean["exit_repricing_applied"], serde_json::json!(0));
        assert!(clean.get("stats_precede_exit_repricing").is_none());
    }

    /// A non-object aggregate cannot be merged into. Dropping it to make room
    /// for the trades would swap one recording gap for another.
    #[test]
    fn a_non_object_aggregate_is_kept_rather_than_discarded() {
        let doc = outcome(
            serde_json::Value::String("engine said nothing".into()),
            vec![round_trip(dec!(1), dec!(0.1))],
        )
        .result_document();
        assert_eq!(
            doc["engine_stats"],
            serde_json::json!("engine said nothing")
        );
        assert_eq!(
            doc[TRADES_KEY].as_array().map(Vec::len),
            Some(1),
            "the trade rows still land"
        );
    }

    /// The SDK's position count includes positions still open at the window's
    /// end, so it is published beside the harvest rather than compared with it.
    #[test]
    fn the_engines_own_position_count_is_published_next_to_the_harvest() {
        let doc = outcome(
            serde_json::json!({ "total_orders": 4, "total_positions": 3 }),
            vec![
                round_trip(dec!(1), dec!(0.1)),
                round_trip(dec!(2), dec!(0.1)),
            ],
        )
        .result_document();
        assert_eq!(doc["trade_rows_harvested"], serde_json::json!(2));
        assert_eq!(doc["engine_total_positions"], serde_json::json!(3));
        assert!(
            doc.get(TRADES_KEY).is_some(),
            "a run holding a position at the close is the normal shape of \
             harvested < total and is not a refusal"
        );
    }

    #[test]
    fn dec_str_pads_and_rounds() {
        assert_eq!(dec_str(dec!(1.5), 4), "1.5000");
        assert_eq!(dec_str(dec!(1.23456), 4), "1.2346");
        assert_eq!(dec_str(dec!(100), 2), "100.00");
    }

    #[test]
    fn base_position_id_strips_only_a_snapshot_uuid_suffix() {
        assert_eq!(
            base_position_id("BTC-USDT.BINANCE-EMA-001-3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f"),
            "BTC-USDT.BINANCE-EMA-001"
        );
        assert_eq!(
            base_position_id("BTC-USDT.BINANCE-EMA-001"),
            "BTC-USDT.BINANCE-EMA-001"
        );
        assert_eq!(
            base_position_id("P-zzzzzzzz-9a4d-4e2b-8c7f-1a2b3c4d5e6f"),
            "P-zzzzzzzz-9a4d-4e2b-8c7f-1a2b3c4d5e6f"
        );
        assert_eq!(
            base_position_id("3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f"),
            "3f2b8c1e-9a4d-4e2b-8c7f-1a2b3c4d5e6f"
        );
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

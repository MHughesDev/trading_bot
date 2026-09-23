//! The simulator honours the declared exit policy (ADR-P6-11).
//!
//! Every expectation here is hand-computed from the bars in the test and stated
//! in the assertion message, because the thing being tested is arithmetic the
//! engine could get subtly wrong in the strategy's favour without failing
//! anything else.
//!
//! The fixtures all use the same shape: one entry signal that is true from the
//! first bar, so the rising edge opens exactly one position at the close of bar
//! 0 and never re-enters. What differs is the exit policy and the bars.

use std::sync::Arc;

use backtest::sim::{
    conservative_exit_fill, run_simulation_detailed, InstrumentPrecisions, SimulationInputs,
};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::exits::PositionSide;
use domain::strategy_def::StrategyDefinition;
use nautilus_backtest::sdk::SimulationControl;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// 61 bp: the fallback schedule's 60 bp taker plus 1 bp of half-spread. Both
/// legs of every trade pay it, and nothing in the platform charges it twice.
const TAKER_RATE: Decimal = dec!(0.0061);

fn ts_ns(i: i64) -> i64 {
    i * 60_000_000_000
}

fn bar(i: i64, high: Decimal, low: Decimal, close: Decimal) -> LoadedBar {
    LoadedBar {
        ts_ns: ts_ns(i),
        open: close,
        high,
        low,
        close,
        volume: dec!(10),
        trade_count: 1,
        ..Default::default()
    }
}

/// A flat bar at `p` — used for the bars where nothing is meant to happen.
fn flat(i: i64, p: Decimal) -> LoadedBar {
    bar(i, p, p, p)
}

/// A definition that goes long one unit on the first bar and carries `exits`.
///
/// `bar('close') > 0` is true on every bar, so the rising-edge semantics the
/// simulator gives entries fire it exactly once.
fn long_with_exits(exits: &str) -> StrategyDefinition {
    let version = if exits.is_empty() { "1.0" } else { "1.3" };
    let exits_block = if exits.is_empty() {
        String::new()
    } else {
        format!(r#""exits": {exits},"#)
    };
    let json = format!(
        r#"{{
            "strategy_id": "always_long_v1",
            "definition_version": "{version}",
            "asset_class": "crypto_spot_cex",
            "inputs": [
                {{ "lane": "market.bars.1m", "instrument": "$bound_at_init" }}
            ],
            "nodes": [
                {{ "id": "n1", "type": "condition", "expr": "bar('close') > 0.0" }},
                {{ "id": "n2", "type": "signal", "when": "n1", "emit": "long" }}
            ],
            {exits_block}
            "actions": [
                {{ "on_signal": "long", "type": "place_order",
                   "order": {{ "side": "buy", "size_mode": "fixed", "size": "1" }} }}
            ]
        }}"#
    );
    serde_json::from_str(&json).expect("valid fixture definition")
}

fn run(def: StrategyDefinition, bars: Vec<LoadedBar>) -> backtest::sim::DetailedOutcome {
    let inputs = SimulationInputs {
        definition: def,
        instrument_id: "BTC-USD".into(),
        venue_id: "coinbase_advanced".into(),
        asset_class: "crypto_spot_cex".into(),
        timeframe: Timeframe::Minutes1,
        quote_currency: "USD".into(),
        initial_balance: Decimal::from(10_000),
        precisions: Some(InstrumentPrecisions { price: 2, size: 4 }),
        sim_start_ns: 0,
        bars,
        features: Vec::new(),
    };
    let control: Arc<SimulationControl> = SimulationControl::new();
    run_simulation_detailed(inputs, &control).expect("simulation runs")
}

// ── the three exit kinds, each at the bar the arithmetic says ────────────────

/// Entry at 100 on bar 0's close; a 2% stop rests at 98. Bar 2 trades down to
/// 97 and closes back up at 100.80.
///
/// The bar it fires on is bar 2 and the price it fires at is 98 — **not**
/// 100.80, which is what the venue's market order actually paid and would book
/// a gain on an order that lost money.
#[test]
fn a_declared_stop_loss_fires_on_the_breaching_bar_and_fills_at_its_level() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "stop_loss", "fraction": "0.02" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(101), dec!(99.5), dec!(100.5)),
            bar(2, dec!(101), dec!(97), dec!(100.8)),
            flat(3, dec!(100)),
            flat(4, dec!(100)),
        ],
    );

    assert_eq!(
        out.exit_fills.len(),
        1,
        "one declared exit fired: {:?}",
        out.exit_fills
    );
    let fill = &out.exit_fills[0];
    assert_eq!(fill.rule, "stop_loss");
    assert_eq!(fill.level, dec!(98), "2% below the 100 entry");
    assert_eq!(fill.venue_fill, dec!(100.8), "what the market order paid");
    assert_eq!(
        fill.charged,
        dec!(98),
        "the worse of the two, never the close"
    );
    assert_eq!(
        fill.bars_held, 2,
        "bars 1 and 2; the entry bar is not one of them"
    );

    assert_eq!(out.trades.len(), 1, "one round trip");
    let t = &out.trades[0];
    assert_eq!(t.entry_price, dec!(100), "the entry fills at bar 0's close");
    assert_eq!(t.exit_price, dec!(98));
    assert_eq!(t.exit_time.timestamp_nanos_opt(), Some(ts_ns(2)));
    assert!(
        t.pnl < Decimal::ZERO,
        "a stop is a loss: -2.00 of price and ~1.22 of fees, got {}",
        t.pnl
    );
}

/// Entry at 100; a 5% target rests at 105. Bar 2 trades up to 106 and closes at
/// 105.50. The target fills at 105 — the level it rested at, not the bar's high
/// and not the close, either of which would be a windfall.
#[test]
fn a_declared_take_profit_fires_at_its_level_and_not_at_the_bar_extreme() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "take_profit", "fraction": "0.05" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(101), dec!(99.9), dec!(100.5)),
            bar(2, dec!(106), dec!(100.4), dec!(105.5)),
            flat(3, dec!(105)),
        ],
    );

    assert_eq!(out.exit_fills.len(), 1, "{:?}", out.exit_fills);
    let fill = &out.exit_fills[0];
    assert_eq!(fill.rule, "take_profit");
    assert_eq!(
        fill.charged,
        dec!(105),
        "no windfall from the 106 high or the 105.50 close"
    );

    let t = &out.trades[0];
    assert_eq!(t.exit_price, dec!(105));
    assert_eq!(t.exit_time.timestamp_nanos_opt(), Some(ts_ns(2)));
    assert!(
        t.pnl > Decimal::ZERO,
        "+5.00 of price against ~1.25 of fees, got {}",
        t.pnl
    );
}

/// `max_holding_bars: 3` with the entry at the close of bar 0 closes on bar 3 —
/// the third bar the position was actually exposed for. Firing on bar 4 would
/// be a strategy that held longer than it declared, which is what
/// `ExitTracker::opened` (the open-fill convention) would have produced here.
///
/// A time exit has no level to rest at, so it fills wherever the probe found
/// it, and under `WorstFirst` the first probe is the adverse extreme: bar 3's
/// low of 99.
#[test]
fn a_declared_time_exit_fires_on_the_declared_bar() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "max_holding_bars", "bars": 3 } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            flat(1, dec!(100)),
            flat(2, dec!(100)),
            bar(3, dec!(102), dec!(99), dec!(101)),
            flat(4, dec!(101)),
        ],
    );

    assert_eq!(out.exit_fills.len(), 1, "{:?}", out.exit_fills);
    let fill = &out.exit_fills[0];
    assert_eq!(fill.rule, "max_holding_bars");
    assert_eq!(fill.bars_held, 3, "bars 1, 2 and 3");
    assert_eq!(
        fill.charged,
        dec!(99),
        "the adverse extreme, not the 101 close"
    );

    let t = &out.trades[0];
    assert_eq!(
        t.exit_time.timestamp_nanos_opt(),
        Some(ts_ns(3)),
        "bar 3, not bar 4"
    );
    assert_eq!(t.exit_price, dec!(99));
}

// ── the intrabar rule, which is the reason any of this is delicate ───────────

/// Bar 1's low breaches the 98 stop **and** its high clears the 105 target.
/// OHLCV cannot say which came first, and the target is declared first so that
/// nothing in the policy's own ordering rescues the optimistic answer.
///
/// The run must book the stop. A simulator that books the target here reports a
/// +5% win on every such bar, which over a long backtest is worth more than
/// most real edges.
#[test]
fn a_bar_that_breached_both_books_the_stop_and_never_the_target() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [
                   { "rule": "take_profit", "fraction": "0.05" },
                   { "rule": "stop_loss", "fraction": "0.02" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(106), dec!(97), dec!(104)),
            flat(2, dec!(104)),
        ],
    );

    assert_eq!(out.exit_fills.len(), 1, "{:?}", out.exit_fills);
    let fill = &out.exit_fills[0];
    assert_eq!(
        fill.rule, "stop_loss",
        "the adverse extreme is probed first, whatever order the rules are declared in"
    );
    assert_eq!(fill.charged, dec!(98));

    let t = &out.trades[0];
    assert_ne!(
        t.exit_price,
        dec!(105),
        "the target is the optimistic resolution"
    );
    assert_ne!(
        t.exit_price,
        dec!(106),
        "the bar high is worse than optimistic"
    );
    assert_ne!(
        t.exit_price,
        dec!(104),
        "the close is the venue's fill, better than the stop"
    );
    assert_eq!(t.exit_price, dec!(98));
    assert!(
        t.pnl < Decimal::ZERO,
        "the bar that breached both must resolve to a loss, got {}",
        t.pnl
    );
}

/// The unit form of the same rule, stated for both sides so a sign error in one
/// of them cannot hide behind a long-only fixture.
#[test]
fn the_conservative_fill_never_returns_the_better_of_the_two_prices() {
    // Closing a long is a sale: lower is worse.
    assert_eq!(
        conservative_exit_fill(PositionSide::Long, dec!(98), dec!(104)),
        dec!(98)
    );
    assert_eq!(
        conservative_exit_fill(PositionSide::Long, dec!(98), dec!(95)),
        dec!(95)
    );
    // Closing a short is a purchase: higher is worse.
    assert_eq!(
        conservative_exit_fill(PositionSide::Short, dec!(102), dec!(96)),
        dec!(102)
    );
    assert_eq!(
        conservative_exit_fill(PositionSide::Short, dec!(102), dec!(110)),
        dec!(110)
    );
}

// ── causality ───────────────────────────────────────────────────────────────

/// The entry fills at the close of bar 0, so bar 0's low of 90 is a price the
/// position was never in the market for. A simulator that evaluates the exit
/// policy against the entry bar stops this position out on a move that happened
/// before it existed.
#[test]
fn the_entry_bar_cannot_stop_out_a_position_opened_at_its_close() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "stop_loss", "fraction": "0.02" } ] }"#,
        ),
        vec![
            bar(0, dec!(100), dec!(90), dec!(100)),
            flat(1, dec!(100)),
            flat(2, dec!(100)),
        ],
    );

    assert!(
        out.exit_fills.is_empty(),
        "bar 0's 90 low happened before the fill at its close: {:?}",
        out.exit_fills
    );
    assert!(out.trades.is_empty(), "nothing closed");
}

// ── blast radius ────────────────────────────────────────────────────────────

/// A pre-1.3 definition declares no exit policy, and the bars below would have
/// stopped out any strategy that did. Nothing must change for it.
#[test]
fn a_definition_with_no_declared_exits_is_untouched() {
    let out = run(
        long_with_exits(""),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(101), dec!(90), dec!(100.5)),
            bar(2, dec!(106), dec!(97), dec!(104)),
            flat(3, dec!(104)),
        ],
    );

    assert!(out.exit_fills.is_empty(), "no policy, no exits");
    assert!(
        out.trades.is_empty(),
        "the position is still open at the end of the run"
    );
}

/// `RiskLayerOnly` is a declared delegation, not a rule set. It closes nothing
/// here, and it must not be silently upgraded into one that does.
#[test]
fn risk_layer_only_closes_nothing_in_the_simulator() {
    let out = run(
        long_with_exits(r#"{ "mode": "risk_layer_only" }"#),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(101), dec!(90), dec!(100.5)),
            flat(2, dec!(100)),
        ],
    );

    assert!(out.exit_fills.is_empty());
    assert!(out.trades.is_empty());
}

/// Found by the exit adjustment multiplying by it and doing nothing: a closed
/// nautilus `Position` reports `quantity` zero, because that field is the
/// *open* size. Every trade this executor produced carried a size of zero, and
/// nothing downstream had noticed because no metric divided by it.
///
/// The fixture buys exactly one unit, so the trade must say one.
#[test]
fn a_closed_trade_reports_the_size_it_actually_traded() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "take_profit", "fraction": "0.05" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(106), dec!(100), dec!(105.5)),
            flat(2, dec!(105)),
        ],
    );

    assert_eq!(
        out.trades[0].qty,
        Decimal::ONE,
        "the order was for one unit"
    );
}

// ── costs ───────────────────────────────────────────────────────────────────

/// Both legs are charged through the one `FeeBook`. `coinbase_advanced` with no
/// tier resolves to the most expensive schedule the book holds for that venue —
/// 60 bp taker plus 1 bp of half-spread — and the fallback says so in its tier
/// name.
///
/// Hand-computed: entry 1 × 100.00 × 0.0061 = 0.61, exit 1 × 105.50 × 0.0061 =
/// 0.643…, so the round trip is ~1.25 of a 10 000 account. An exit that paid
/// nothing would come in at 0.61.
#[test]
fn an_exit_fill_is_charged_through_the_same_fee_book_as_the_entry() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "take_profit", "fraction": "0.05" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(106), dec!(100), dec!(105.5)),
            flat(2, dec!(105)),
        ],
    );

    let t = &out.trades[0];
    let entry_leg = t.entry_price * t.qty * TAKER_RATE;
    let exit_leg = out.exit_fills[0].venue_fill * t.qty * TAKER_RATE;
    let expected = entry_leg + exit_leg;

    assert!(
        t.costs_paid > entry_leg,
        "the exit leg paid nothing: costs {} vs the entry leg's {entry_leg} alone",
        t.costs_paid
    );
    assert!(
        (t.costs_paid - expected).abs() < dec!(0.02),
        "expected ~{expected} of commission across both legs, got {}",
        t.costs_paid
    );

    assert_eq!(out.fee_schedule.venue, "coinbase_advanced");
    assert_eq!(
        out.fee_schedule.tier, "intro/unknown-fallback",
        "the fallback says so"
    );
}

/// The assumptions are on the result, not in a log line. A reader of stored
/// evidence has to be able to tell a run resolved pessimistically from one
/// whose strategy simply never breached a level.
#[test]
fn the_run_records_the_execution_assumptions_it_was_computed_under() {
    let out = run(
        long_with_exits(
            r#"{ "mode": "rules", "rules": [ { "rule": "stop_loss", "fraction": "0.02" } ] }"#,
        ),
        vec![
            flat(0, dec!(100)),
            bar(1, dec!(101), dec!(97), dec!(100.9)),
            flat(2, dec!(100)),
        ],
    );

    let flags = out.execution_assumptions();
    let codes: Vec<&str> = flags.iter().map(|f| f.code.as_str()).collect();
    assert!(codes.contains(&"exec.fee_schedule"), "{codes:?}");
    assert!(codes.contains(&"exec.intrabar_resolution"), "{codes:?}");

    let intrabar = flags
        .iter()
        .find(|f| f.code == "exec.intrabar_resolution")
        .unwrap();
    assert!(
        intrabar.detail.contains("worst_first"),
        "the assumption must name itself: {}",
        intrabar.detail
    );
}

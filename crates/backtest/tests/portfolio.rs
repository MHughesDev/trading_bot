//! Set U U-6.5: a portfolio backtest of a selection-bound deployment.
//!
//! Acceptance: an instrument enters and leaves the portfolio as membership
//! changes, with trades only while a member; P&L aggregates per numeraire; the
//! run's trade list attributes every trade to its instance.

use std::collections::BTreeMap;
use std::sync::Arc;

use backtest::portfolio::{run_portfolio, PortfolioOutcome, PortfolioRun};
use backtest::runner;
use backtest::store::LoadedBar;
use chrono::{DateTime, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use dataplane::universe::MembershipEvent;
use dataplane::InstrumentKey;
use domain::instrument::AssetClass;
use domain::strategy_graph::deployment::{OnRemoval, Rebalance, Weighting};
use domain::strategy_graph::{MissingReason, StrategyGraph, Value};
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::allocation::AllocationChange;
use strategy_engine::env::NoModels;
use strategy_engine::{InstrumentSpec, Membership, StdRegistries, VenueCaps};

const HOUR: i64 = 3_600_000_000_000;
const DAY: i64 = 24 * HOUR;
const HOURS: usize = 240;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
}

fn t0_ns() -> i64 {
    t0().timestamp_nanos_opt().unwrap()
}

fn d(x: f64) -> Decimal {
    Decimal::from_f64_retain((x * 100.0).round() / 100.0).unwrap()
}

fn bars(period: i64, closes: &[f64]) -> Vec<LoadedBar> {
    let mut prev = closes[0];
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let open_ns = t0_ns() + i as i64 * period;
            let (o, h, l) = (prev, prev.max(c) + 0.2, prev.min(c) - 0.2);
            prev = c;
            LoadedBar {
                ts_ns: open_ns + period,
                open_ns,
                knowledge_ns: open_ns + period,
                open: d(o),
                high: d(h),
                low: d(l),
                close: d(c),
                volume: dec!(1000),
                trade_count: 10,
                quality_flags: QualityFlags::default(),
            }
        })
        .collect()
}

/// A falls through B on day 4; C trades highest of all from the start, but is
/// only admitted to the universe on day 7.
fn hourly(m: &str) -> Vec<f64> {
    (0..HOURS)
        .map(|i| {
            let x = i as f64;
            match m {
                "A" => 150.0 - 0.4 * x,
                "B" => 100.0 + 0.1 * x,
                _ => 200.0 + (x / 5.0).sin(),
            }
        })
        .collect()
}

/// Daily bars closing at the same instants as every 24th hourly bar.
fn daily(m: &str) -> Vec<f64> {
    hourly(m).into_iter().skip(23).step_by(24).collect()
}

#[allow(clippy::needless_pass_by_value)]
fn graph(v: serde_json::Value) -> StrategyGraph {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}\n{v:#}"))
}

fn highest_price() -> StrategyGraph {
    graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "highest",
      "asset_class": "crypto_spot_cex", "timeframe": "1d",
      "universe": {"kind": "named", "name": "test"},
      "nodes": [{"id":"p", "op":"bar", "args":{"field":{"str":"close"}}}],
      "select": {"score": "p", "top_n": 1}
    }))
}

fn always_long() -> StrategyGraph {
    graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "hold_long",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"up",    "op":"gt",  "args":{"a":"close","b":0}},
        {"id":"entry", "op":"and", "args":{"a":"up","b":"state.flat"}}
      ],
      "decide": {"rules": [
        {"when":"entry","then":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}}}
      ]}
    }))
}

fn membership() -> Membership {
    let k = InstrumentKey;
    Membership::logged(
        vec![
            MembershipEvent::added("test", k(1), t0(), "seed"),
            MembershipEvent::added("test", k(2), t0(), "seed"),
            MembershipEvent::added("test", k(3), t0() + chrono::Duration::days(7), "listed"),
        ],
        BTreeMap::from([(1, "A".into()), (2, "B".into()), (3, "C".into())]),
    )
    .unwrap()
}

fn spec(m: &str) -> (InstrumentSpec, InstrumentLimits) {
    (
        InstrumentSpec {
            instrument: m.into(),
            lot_size: dec!(0.0001),
            tick_size: dec!(0.01),
            min_qty: dec!(0.0001),
            caps: VenueCaps::all(),
        },
        InstrumentLimits {
            tick_size: dec!(0.01),
            lot_size: Decimal::ZERO,
        },
    )
}

fn run(on_removal: OnRemoval) -> PortfolioOutcome {
    run_with(on_removal, Rebalance::OnSelectionRun, &highest_price())
}

fn run_with(
    on_removal: OnRemoval,
    rebalance: Rebalance,
    selection: &StrategyGraph,
) -> PortfolioOutcome {
    run_graphs(on_removal, rebalance, selection, &always_long())
}

fn run_graphs(
    on_removal: OnRemoval,
    rebalance: Rebalance,
    selection: &StrategyGraph,
    trading: &StrategyGraph,
) -> PortfolioOutcome {
    let reg = StdRegistries::new();
    let none = backtest::run::ParamMap::new();
    let trading = runner::compile_for_run(trading, &none, &reg).unwrap();
    let selection = runner::compile_for_run(selection, &none, &reg).unwrap();
    let members = ["A", "B", "C"];
    let r = PortfolioRun {
        trading: Arc::new(trading),
        selection: Arc::new(selection),
        membership: membership(),
        top_n: 1,
        on_removal,
        weighting: Weighting::Equal,
        rebalance,
        trading_bars: members
            .iter()
            .map(|m| ((*m).to_string(), vec![bars(HOUR, &hourly(m))]))
            .collect(),
        selection_bars: members
            .iter()
            .map(|m| ((*m).to_string(), vec![bars(DAY, &daily(m))]))
            .collect(),
        start_ns: t0_ns() + 2 * DAY,
        specs: members
            .iter()
            .map(|m| ((*m).to_string(), spec(m)))
            .collect(),
        asset_class: AssetClass::CryptoSpotCex,
        limits: GlobalRiskLimits::default(),
        capital: dec!(10_000),
        fill_policy: FillPolicy::NextBarOpen,
        strategy_id: "portfolio-test".into(),
        sims: SimulatorSet::realistic(),
    };
    run_portfolio(r, &mut NoModels).unwrap_or_else(|e| panic!("{e}"))
}

fn day_of(ns: i64) -> i64 {
    (ns - t0_ns()) / DAY
}

/// Instrument → (entered at, left at) intervals, from the journaled changes.
fn held(o: &PortfolioOutcome) -> BTreeMap<String, Vec<(i64, Option<i64>)>> {
    let mut out: BTreeMap<String, Vec<(i64, Option<i64>)>> = BTreeMap::new();
    for (t, c) in &o.changes {
        match c {
            AllocationChange::Entered { instrument, .. } => {
                out.entry(instrument.clone()).or_default().push((*t, None));
            }
            AllocationChange::Left { instrument, .. } => {
                if let Some(last) = out.get_mut(instrument).and_then(|v| v.last_mut()) {
                    last.1 = Some(*t);
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn members_enter_and_leave_as_the_selection_changes_and_trade_only_while_members() {
    let o = run(OnRemoval::Close);
    let h = held(&o);
    let days = |m: &str| -> Vec<(i64, Option<i64>)> {
        h[m].iter()
            .map(|(a, b)| (day_of(*a), b.map(day_of)))
            .collect()
    };
    assert_eq!(
        days("A"),
        [(3, Some(5))],
        "A leads until it falls through B on day 4"
    );
    assert_eq!(days("B"), [(5, Some(7))]);
    assert_eq!(
        days("C"),
        [(7, None)],
        "C prices highest from day 0 but is only a member from day 7"
    );
    assert!(o.universe_point_in_time);
    assert!(o.skipped.is_empty(), "{:?}", o.skipped);

    // Every trade inside its instrument's membership — entered on the bar
    // after the rebalance admits it, flattened on the bar after the one that
    // drops it.
    assert!(!o.trades.is_empty());
    for t in &o.trades {
        let spans = &h[&t.trade.symbol];
        let entry = t.trade.entry_time.timestamp_nanos_opt().unwrap();
        let exit = t.trade.exit_time.timestamp_nanos_opt().unwrap();
        assert!(
            spans
                .iter()
                .any(|(from, to)| entry > *from && to.is_none_or(|to| exit <= to + HOUR)),
            "{} traded outside its membership: {:?} vs {spans:?}",
            t.trade.symbol,
            (t.trade.entry_time, t.trade.exit_time)
        );
    }

    // Decisions likewise: no instance decides for an instrument it does not hold.
    for (instance, trace) in &o.traces {
        let m = instance.split('#').next().unwrap();
        assert!(
            h[m].iter()
                .any(|(from, to)| trace.at_ns >= *from && to.is_none_or(|to| trace.at_ns < to)),
            "{instance} decided at day {} outside its membership",
            day_of(trace.at_ns)
        );
    }
}

#[test]
fn every_trade_is_attributed_to_its_instance_and_pnl_is_summed_per_numeraire() {
    let o = run(OnRemoval::Close);
    let closed: Vec<&str> = o.trades.iter().map(|t| t.trade.symbol.as_str()).collect();
    assert_eq!(
        closed,
        ["A", "B"],
        "A and B were flattened; C is still held"
    );
    for t in &o.trades {
        assert_eq!(
            t.instance,
            format!("{}#1", t.trade.symbol),
            "the instance that opened it"
        );
    }
    let total: Decimal = o.trades.iter().map(|t| t.trade.pnl).sum();
    assert_eq!(o.pnl_by_numeraire.keys().collect::<Vec<_>>(), ["USD"]);
    assert_eq!(o.pnl_by_numeraire["USD"], total);
    assert!(o.trades[0].trade.pnl < Decimal::ZERO, "A fell while held");
    // One account: the equity curve is the whole portfolio's.
    assert!(!o.equity.is_empty());
    assert_eq!(o.selections.len(), 8, "days 3 through 10");
}

#[test]
fn hold_keeps_the_leavers_position_and_stops_deciding_for_it() {
    let o = run(OnRemoval::Hold);
    assert!(
        o.trades.is_empty(),
        "nothing was flattened, so no round trip closed"
    );
    let h = held(&o);
    let a_left = h["A"][0].1.unwrap();
    assert!(
        o.traces
            .iter()
            .all(|(i, t)| !i.starts_with('A') || t.at_ns < a_left),
        "A's instance stopped at the rebalance that dropped it"
    );
}

#[test]
fn let_strategy_exit_keeps_the_instance_and_its_capital_until_its_own_rules_exit() {
    // This trading strategy never exits on its own, so A's instance runs to the
    // end and keeps all of the deployment's capital: B and C, picked later, wait
    // for capital that never frees rather than starting with none.
    let o = run(OnRemoval::LetStrategyExit);
    let kinds: Vec<(i64, String)> = o
        .changes
        .iter()
        .map(|(t, c)| {
            let k = match c {
                AllocationChange::Entered { .. } => "entered",
                AllocationChange::Exiting { .. } => "exiting",
                AllocationChange::Left { .. } => "left",
                AllocationChange::Reweighted { .. } => "reweighted",
            };
            (day_of(*t), format!("{k} {}", c.instrument()))
        })
        .collect();
    assert_eq!(
        kinds,
        [(3, "entered A".to_string()), (5, "exiting A".to_string())]
    );
    let last_a = o
        .traces
        .iter()
        .filter(|(i, _)| i.starts_with('A'))
        .map(|(_, t)| t.at_ns)
        .max()
        .unwrap();
    assert!(
        day_of(last_a) >= 9,
        "A's instance still decides after it left the selection"
    );
    assert!(
        o.traces.iter().all(|(i, _)| i.starts_with('A')),
        "nobody else was started"
    );
    assert_eq!(o.selections.len(), 8, "the selection kept running");
}

#[test]
fn a_scheduled_selection_runs_at_its_scheduled_instants() {
    let mut g = serde_json::to_value(highest_price()).unwrap();
    g["triggers"] = serde_json::json!({"bar_close": false, "schedule": "0 12 * * *"});
    let g: StrategyGraph = serde_json::from_value(g).unwrap();
    let o = run_with(OnRemoval::Close, Rebalance::OnSelectionRun, &g);
    assert!(!o.changes.is_empty());
    for (t, _) in &o.changes {
        assert_eq!(
            (t - t0_ns()).rem_euclid(DAY),
            12 * HOUR,
            "every rebalance at noon UTC"
        );
    }
    assert_eq!(o.selections.len(), 8, "noon on days 2 through 9");
}

#[test]
fn a_rebalance_schedule_applies_membership_only_when_it_fires() {
    // 2025-01-01 is a Wednesday: the only Monday in the window is the 6th
    // (day 5), when B leads. The daily selections before it change nothing.
    let o = run_with(
        OnRemoval::Close,
        Rebalance::Schedule {
            cron: "0 0 * * 1".into(),
        },
        &highest_price(),
    );
    let changes: Vec<(i64, &str)> = o
        .changes
        .iter()
        .map(|(t, c)| (day_of(*t), c.instrument()))
        .collect();
    assert_eq!(changes, [(5, "B")], "{changes:?}");
    assert_eq!(o.selections.len(), 8, "the selection still ran daily");
    assert!(o
        .traces
        .iter()
        .all(|(i, t)| i == "B#1" && day_of(t.at_ns) >= 5));
}

// ── U-6.6 ────────────────────────────────────────────────────────────────────

/// The selection that picks the highest close, and says what that close was.
fn highest_with_its_close() -> StrategyGraph {
    graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "highest_w",
      "asset_class": "crypto_spot_cex", "timeframe": "1d",
      "universe": {"kind": "named", "name": "test"},
      "nodes": [{"id":"p", "op":"bar", "args":{"field":{"str":"close"}}}],
      "select": {"score": "p", "top_n": 1, "attributes": {"w": "p"}}
    }))
}

/// Holds long, and reads what its selection said about its instrument.
fn long_reading(selection: &str) -> StrategyGraph {
    graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "reads_w",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"w",     "op":"selection_attr", "args":{"selection":{"str":selection},"attr":{"str":"w"}}},
        {"id":"up",    "op":"gt",  "args":{"a":"close","b":0}},
        {"id":"entry", "op":"and", "args":{"a":"up","b":"state.flat"}}
      ],
      "decide": {"rules": [
        {"when":"entry","then":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}}}
      ]}
    }))
}

#[test]
fn a_member_reads_the_attribute_its_latest_selection_gave_it() {
    let o = run_graphs(
        OnRemoval::Close,
        Rebalance::OnSelectionRun,
        &highest_with_its_close(),
        &long_reading("highest_w"),
    );
    assert!(!o.traces.is_empty());
    let mut read = 0;
    for (instance, trace) in &o.traces {
        if trace.cause != strategy_engine::trace::Cause::BarClose {
            continue;
        }
        let member = instance.split('#').next().unwrap();
        // The latest selection at or before this decision.
        let latest = o
            .selections
            .iter()
            .filter(|s| s.as_of.timestamp_nanos_opt().unwrap() <= trace.at_ns)
            .next_back()
            .expect("a member decides only after a selection");
        let given = latest
            .selected
            .iter()
            .find(|s| s.instrument == member)
            .and_then(|s| s.attributes.get("w"))
            .cloned();
        match given {
            Some(Value::Num(x)) => {
                assert_eq!(trace.values["w"], Value::num(x.get(), "w"), "{instance} at {}", trace.at_ns);
                read += 1;
            }
            // An exiting member its latest selection no longer names reads
            // what the selection last said about it; nothing to compare here.
            _ => {}
        }
    }
    assert!(read > 10, "members read their attribute ({read} decisions)");
}

#[test]
fn a_member_naming_another_selection_reads_missing_with_the_reason() {
    let o = run_graphs(
        OnRemoval::Close,
        Rebalance::OnSelectionRun,
        &highest_with_its_close(),
        &long_reading("some_other_screener"),
    );
    let (_, trace) = o
        .traces
        .iter()
        .find(|(_, t)| t.cause == strategy_engine::trace::Cause::BarClose)
        .unwrap();
    let Value::Missing(MissingReason::NoData { what }) = &trace.values["w"] else {
        panic!("{:?}", trace.values["w"]);
    };
    assert!(what.contains("highest_w") && what.contains("some_other_screener"), "{what}");
}

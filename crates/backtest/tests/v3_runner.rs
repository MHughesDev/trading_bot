//! Set U Phase 3 acceptance: v3 strategies backtest through the one strategy
//! engine, on the simulated venue, behind the risk gate, in the shape Set J
//! judges (U-3.2, U-3.3).

use std::sync::{Arc, Mutex};

use backtest::run::{
    Backtest, ComputeCost, DataSlice, EvalResolution, FillModel, InMemoryRunStore, MetricKind,
    ParamMap, RunConfig, RunConfigBuilder, RunDispatch, RunExecutor, RunResult,
};
use backtest::runner::{self, V3Run, RUNNER_TAG};
use backtest::store::LoadedBar;
use backtest::study::{SelectionRule, StudyBudget, StudyConfig, StudyEngine, StudyKind, VarySpec};
use chrono::{DateTime, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use domain::instrument::AssetClass;
use domain::strategy_graph::{StrategyGraph, Value};
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use ledger::{DispatchContext, InMemoryLedger};
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::trace::Cause;
use strategy_engine::{InstrumentSpec, VenueCaps};

const HOUR: i64 = 3_600_000_000_000;
const DAY: i64 = 24 * HOUR;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
}

fn t0_ns() -> i64 {
    t0().timestamp_nanos_opt().unwrap()
}

fn d(x: f64) -> Decimal {
    Decimal::from_f64_retain((x * 100.0).round() / 100.0).unwrap()
}

/// Bars of `period` ns whose closes are `closes`, each opening at the previous
/// close.
fn bars(start_ns: i64, period: i64, closes: &[f64]) -> Vec<LoadedBar> {
    let mut prev = closes[0];
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let open_ns = start_ns + i as i64 * period;
            let (o, h, l) = (prev, prev.max(c) + 0.3, prev.min(c) - 0.3);
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

/// An oscillating series with enough swing for EMA crosses and bracket legs.
fn wave(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = i as f64;
            100.0 + 8.0 * (x / 12.0).sin() + 3.0 * (x / 5.0).cos()
        })
        .collect()
}

#[allow(clippy::needless_pass_by_value)]
fn graph(v: serde_json::Value) -> StrategyGraph {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}\n{v:#}"))
}

fn ema_cross(extra_rules: serde_json::Value) -> StrategyGraph {
    let mut rules = vec![
        serde_json::json!({"when":"entry","then":{"kind":"target","size":{"mode":"units","x":10}}}),
        serde_json::json!({"when":"leave","then":{"kind":"close"}}),
    ];
    if let serde_json::Value::Array(more) = extra_rules {
        rules = more;
    }
    graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "ema_cross",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "params": {"fast": {"type":"int","default":5,"min":2,"max":20}},
      "nodes": [
        {"id":"f",    "op":"ema",           "args":{"x":"close", "n":"$fast"}},
        {"id":"s",    "op":"ema",           "args":{"x":"close", "n":20}},
        {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
        {"id":"entry","op":"and",           "args":{"a":"up", "b":"state.flat"}},
        {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
        {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
      ],
      "decide": {"rules": rules}
    }))
}

fn spec(instrument: &str) -> InstrumentSpec {
    InstrumentSpec {
        instrument: instrument.into(),
        lot_size: dec!(0.0001),
        tick_size: dec!(0.01),
        min_qty: dec!(0.0001),
        caps: VenueCaps::all(),
    }
}

fn v3_run(
    g: &StrategyGraph,
    params: &ParamMap,
    inputs: Vec<Vec<LoadedBar>>,
    start_ns: i64,
) -> V3Run {
    let plan = runner::compile_for_run(g, params, &strategy_engine::StdRegistries::new())
        .unwrap_or_else(|e| panic!("{e}"));
    V3Run {
        plan: Arc::new(plan),
        instrument: "ETH-USD".into(),
        asset_class: AssetClass::CryptoSpotCex,
        bars: inputs,
        start_ns,
        spec: spec("ETH-USD"),
        instrument_limits: InstrumentLimits {
            tick_size: dec!(0.01),
            lot_size: Decimal::ZERO,
        },
        limits: GlobalRiskLimits::default(),
        initial_balance: dec!(10_000),
        fill_policy: FillPolicy::NextBarOpen,
        strategy_id: "ema_cross".into(),
        sims: SimulatorSet::realistic(),
    }
}

// ── U-3.2 ────────────────────────────────────────────────────────────────────

#[test]
fn an_indicator_strategy_backtests_end_to_end() {
    let hourly = bars(t0_ns(), HOUR, &wave(500));
    let start = t0_ns() + 60 * HOUR;
    let out = runner::run(
        v3_run(
            &ema_cross(serde_json::Value::Null),
            &ParamMap::new(),
            vec![hourly],
            start,
        ),
        &mut NoModels,
    )
    .expect("runs");

    assert_eq!(out.equity.len(), 440, "one equity point per window bar");
    assert!(
        out.trades.len() >= 5,
        "EMA crosses trade: {} trades",
        out.trades.len()
    );
    for t in &out.trades {
        assert!(t.costs_paid > Decimal::ZERO, "every round trip paid fees");
        assert_eq!(t.qty, dec!(10));
    }
    let m = &out.manifest;
    assert_eq!(m.engine, "strategy-engine/v3");
    assert_eq!(m.fill_policy, "next_bar_open");
    assert!(!m.optimistic_fills);
    assert_eq!(m.decisions, 440, "one decision per window bar close");
    assert!(m.gate_rejections.is_empty());
    assert!(m.journal.iter().any(|j| j.actions > 0));
    assert_eq!(m.journal_digest.len(), 64);
}

#[test]
fn a_bracket_strategy_exits_only_through_its_legs() {
    let rules = serde_json::json!([
      {"when":"entry","then":{"kind":"bracket",
         "entry":{"kind":"target","size":{"mode":"units","x":10}},
         "stop":{"amount":2,"unit":{"unit":"pct"}},
         "take_profit":{"amount":3,"unit":{"unit":"pct"}}}}
    ]);
    let hourly = bars(t0_ns(), HOUR, &wave(600));
    let start = t0_ns() + 60 * HOUR;
    let out = runner::run(
        v3_run(&ema_cross(rules), &ParamMap::new(), vec![hourly], start),
        &mut NoModels,
    )
    .expect("runs");

    assert!(
        !out.trades.is_empty(),
        "the bracket opened and closed positions"
    );
    for t in &out.trades {
        let r = t.exit_price / t.entry_price;
        assert!(
            r <= dec!(0.985) || r >= dec!(1.025),
            "with no exit rule every exit is a leg: entry {} exit {}",
            t.entry_price,
            t.exit_price
        );
    }
    // The legs were placed when the entry filled — a mechanical step on the
    // fill, not a bar-close decision.
    assert!(out
        .manifest
        .journal
        .iter()
        .any(|j| j.cause == "fill" && j.actions >= 2));
    assert!(out
        .traces
        .iter()
        .any(|t| matches!(t.cause, Cause::OrderClosed { .. })
            || matches!(t.cause, Cause::Fill { .. })));
}

#[test]
fn a_daily_input_is_seen_only_once_its_bar_has_closed() {
    // ETH hourly, reading BTC daily. Each BTC daily close is distinct, so the
    // value the strategy saw names the bar it came from.
    let g = graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "pit",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "inputs": [ {"id":"btc_1d", "instrument":{"kind":"fixed","instrument":"BTC-USD"}, "timeframe":"1d"} ],
      "nodes": [
        {"id":"b",  "op":"add", "args":{"a":"btc_1d.close", "b":0}},
        {"id":"ok", "op":"ge",  "args":{"a":"b", "b":0}}
      ],
      "decide": {"rules": [{"when":"ok","then":{"kind":"hold"}}]}
    }));
    let eth = bars(t0_ns(), HOUR, &wave(24 * 6));
    let btc_closes: Vec<f64> = (0..6).map(|i| 1000.0 + f64::from(i)).collect();
    let btc = bars(t0_ns(), DAY, &btc_closes);
    let start = t0_ns();
    let out = runner::run(
        v3_run(&g, &ParamMap::new(), vec![eth, btc.clone()], start),
        &mut NoModels,
    )
    .expect("runs");

    let mut checked = 0;
    for t in out.traces.iter().filter(|t| t.cause == Cause::BarClose) {
        let latest = btc.iter().rev().find(|b| b.ts_ns <= t.at_ns);
        match (latest, t.values.get("b")) {
            (Some(bar), Some(Value::Num(v))) => {
                assert_eq!(
                    d(v.get()),
                    bar.close,
                    "at {} saw the wrong daily bar",
                    t.at_ns
                );
                checked += 1;
            }
            (None, Some(Value::Missing(_))) => {}
            (latest, seen) => panic!("at {}: latest closed {latest:?}, saw {seen:?}", t.at_ns),
        }
    }
    assert!(checked > 100);
    // The hour that closes exactly at a day boundary sees that day's bar.
    let boundary = out
        .traces
        .iter()
        .find(|t| t.at_ns == t0_ns() + 2 * DAY)
        .unwrap();
    assert!(matches!(boundary.values.get("b"), Some(Value::Num(v)) if d(v.get()) == dec!(1001)));
}

// ── through the trial ledger and Set J ───────────────────────────────────────

/// The v3 runner as a [`RunExecutor`] over in-memory bars. Records the trial
/// each run was dispatched under, so a test can find it in the ledger.
struct InMemoryV3 {
    graph: StrategyGraph,
    bars: Vec<LoadedBar>,
    seen: Arc<Seen>,
}

/// What the executor saw, shared with the test.
#[derive(Default)]
struct Seen {
    tickets: Mutex<Vec<uuid::Uuid>>,
    /// Journal digest of every run that completed.
    digests: Mutex<Vec<String>>,
}

impl RunExecutor for InMemoryV3 {
    fn execute(&self, cfg: &RunConfig, ticket: &ledger::TrialTicket) -> RunResult {
        self.seen.tickets.lock().unwrap().push(ticket.trial_id());
        let start = cfg.data_slice.start.timestamp_nanos_opt().unwrap();
        let mut r = v3_run(&self.graph, &cfg.params, vec![self.bars.clone()], start);
        r.fill_policy = runner::fill_policy(cfg.fill_model).unwrap();
        let o = runner::run(r, &mut NoModels).expect("runs");
        self.seen
            .digests
            .lock()
            .unwrap()
            .push(o.manifest.journal_digest.clone());
        runner::to_run_result(cfg, o, ComputeCost::default())
    }
}

fn config(params: ParamMap) -> RunConfig {
    let slice = DataSlice::new(
        "ETH-USD",
        t0() + chrono::Duration::hours(60),
        t0() + chrono::Duration::hours(500),
        EvalResolution::Hour1,
    );
    RunConfigBuilder::new("ema_cross", "rev", slice, "cost:paper", "sz", "snap")
        .params(params)
        .build()
}

fn ctx() -> DispatchContext {
    DispatchContext::human("tenant-v3", "v3", 0.1).with_experiment("v3-exp")
}

fn backtest() -> (
    Backtest<InMemoryRunStore, InMemoryV3, InMemoryLedger>,
    Arc<Seen>,
) {
    let seen = Arc::new(Seen::default());
    let bt = Backtest::new(
        InMemoryRunStore::new(),
        InMemoryV3 {
            graph: ema_cross(serde_json::Value::Null),
            bars: bars(t0_ns(), HOUR, &wave(500)),
            seen: Arc::clone(&seen),
        },
        InMemoryLedger::new(),
    );
    (bt, seen)
}

#[test]
fn every_v3_run_is_in_the_trial_ledger_before_it_computes() {
    let (bt, seen) = backtest();
    let cfg = config(ParamMap::new());
    let ctx = ctx();
    let result = bt
        .run(&RunDispatch::new(&ctx, &cfg, 1.0))
        .expect("dispatches");
    assert_eq!(result.produced_by, RUNNER_TAG);
    let m = result
        .manifest
        .as_ref()
        .expect("a v3 run carries its manifest");
    assert_eq!(m.fill_policy, "next_bar_open");

    // The executor was handed a ticket, and a ticket exists only once its trial
    // row is written: the run's own trial is in the ledger, registered first
    // and settled after.
    let trial = seen.tickets.lock().unwrap()[0];
    let events = bt.ledger().events(trial);
    assert!(events.len() >= 2, "{events:?}");
    assert_eq!(events[0].event_seq, 0);
    assert_eq!(
        events.last().unwrap().run_id.as_deref(),
        Some(cfg.run_id.as_str())
    );
}

#[test]
fn a_set_j_study_accepts_v3_runs_unchanged() {
    let (bt, seen) = backtest();
    let grid: Vec<ParamMap> = [3, 5, 8]
        .iter()
        .map(|f| {
            let mut p = ParamMap::new();
            p.insert("fast".into(), serde_json::json!(f));
            p
        })
        .collect();
    let study = StudyConfig {
        study_id: "v3-sweep".into(),
        kind: StudyKind::ParameterSweep,
        base_config: config(ParamMap::new()),
        vary: VarySpec::Params { grid },
        metric: MetricKind::TotalReturn,
        null_ref: None,
        null: None,
        budget: StudyBudget::default(),
        question: "does the fast period matter".into(),
        selection_rule: SelectionRule::None,
    };
    let result = StudyEngine::run(&study, &bt, &ctx()).expect("the study runs");
    // The study is sealed — its members are not addressable — so read what it
    // reports: three trials added, three surviving members in the distribution.
    assert_eq!(result.trial_delta, 3);
    assert_eq!(result.distribution.dist.len(), 3, "no member failed");
    // Different parameters, different decisions.
    let digests: std::collections::BTreeSet<String> =
        seen.digests.lock().unwrap().iter().cloned().collect();
    assert_eq!(digests.len(), 3);
}

#[test]
fn the_fill_policy_is_part_of_the_run_and_flagged_when_optimistic() {
    let (bt, _) = backtest();
    let ctx = ctx();
    let mut cfg = config(ParamMap::new());
    cfg.fill_model = FillModel::CurrentClose;
    let cfg = cfg.rehashed();
    let r = bt.run(&RunDispatch::new(&ctx, &cfg, 1.0)).unwrap();
    assert!(r.manifest.as_ref().unwrap().optimistic_fills);
    assert!(r
        .integrity_flags
        .iter()
        .any(|f| f.code == "fills.optimistic"));
    assert_ne!(cfg.run_id, config(ParamMap::new()).run_id);
}

// ── U-3.5: preview is a short backtest ───────────────────────────────────────

#[test]
fn a_preview_and_a_backtest_over_the_same_window_decide_identically() {
    use backtest::preview::{preview_v3, PreviewVenue};
    let g = ema_cross(serde_json::Value::Null);
    let hourly = bars(t0_ns(), HOUR, &wave(300));
    let start = t0_ns() + 60 * HOUR;
    let backtest = runner::run(
        v3_run(&g, &ParamMap::new(), vec![hourly.clone()], start),
        &mut NoModels,
    )
    .unwrap();
    let (preview, traces) = preview_v3(
        v3_run(&g, &ParamMap::new(), vec![hourly], start),
        PreviewVenue::Simulated,
        &mut NoModels,
    )
    .unwrap();
    assert_eq!(
        traces, backtest.traces,
        "the same decisions, trace for trace"
    );
    assert_eq!(preview.journal_digest, backtest.manifest.journal_digest);
    assert_eq!(preview.decisions, backtest.manifest.decisions);
    // And it reports no money: the preview type has no equity, trades or P&L.
    let json = serde_json::to_value(&preview).unwrap();
    for absent in ["equity", "trades", "pnl", "equity_curve"] {
        assert!(json.get(absent).is_none(), "a preview reported {absent}");
    }
}

#[test]
fn a_preview_names_the_first_bar_each_rule_fired_and_why_it_held_before() {
    use backtest::preview::{preview_v3, PreviewVenue};
    use strategy_engine::decide::RuleOutcome;
    let g = ema_cross(serde_json::Value::Null);
    // No warm-up before the window: the first decisions run on cold indicators.
    let hourly = bars(t0_ns(), HOUR, &wave(200));
    let (p, traces) = preview_v3(
        v3_run(&g, &ParamMap::new(), vec![hourly], t0_ns()),
        PreviewVenue::Simulated,
        &mut NoModels,
    )
    .unwrap();
    let entry = &p.rules[0];
    let first = traces
        .iter()
        .find(|t| t.rules.get(&0) == Some(&RuleOutcome::Fired))
        .expect("the entry rule fires in the window");
    assert_eq!(
        entry.first_fired_at,
        Some(DateTime::from_timestamp_nanos(first.at_ns))
    );
    assert!(
        entry
            .before_first_fire
            .keys()
            .any(|k| k == "unknown:warming_up"),
        "it held back first because its EMAs were warming up: {:?}",
        entry.before_first_fire
    );
    assert!(entry.before_first_fire.contains_key("not_matched"));
}

#[test]
fn an_intent_only_preview_records_orders_and_never_fills_them() {
    use backtest::preview::{preview_v3, PreviewVenue};
    let g = ema_cross(serde_json::Value::Null);
    let hourly = bars(t0_ns(), HOUR, &wave(300));
    let start = t0_ns() + 60 * HOUR;
    let (sim, _) = preview_v3(
        v3_run(&g, &ParamMap::new(), vec![hourly.clone()], start),
        PreviewVenue::Simulated,
        &mut NoModels,
    )
    .unwrap();
    let (intent, traces) = preview_v3(
        v3_run(&g, &ParamMap::new(), vec![hourly], start),
        PreviewVenue::IntentOnly,
        &mut NoModels,
    )
    .unwrap();
    assert_eq!(intent.venue, PreviewVenue::IntentOnly);
    // Never filled, so never long: the exit rule never fires, and no decision
    // was caused by a fill.
    assert_eq!(intent.rules[1].fired, 0);
    assert!(traces.iter().all(|t| t.cause == Cause::BarClose));
    assert!(intent.rules[0].fired > 0);
    assert!(
        sim.rules[1].fired > 0,
        "with fills, the exit rule does fire"
    );
}

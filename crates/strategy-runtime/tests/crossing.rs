//! Cross detection, end to end, and the distinction that makes it honest:
//! **a cross that could not be evaluated is not a cross that did not happen.**
//!
//! AGENT-011 §3 makes `crossed_above/below` and `rising/falling(x, n)`
//! compiler-provided. Nothing under them existed: `bytecode::Op` had no cross
//! op and no previous-value register, and `strategy-lang` refuses the name by
//! pointing at the level test `a > b`, which is a different claim — true for as
//! long as the fast line stays above the slow one, rather than on the bar it
//! got there.
//!
//! These tests pin three things the runtime now guarantees:
//!
//! 1. a cross fires on the bar it happens and not on the bars after it;
//! 2. a bar where the answer is unknown is *reported*, never emitted as "no
//!    signal", because a backtest that folds the two together loses an entry at
//!    every window boundary and the loss compounds with every re-slice;
//! 3. the bar a cross compares against is the previous **bar** of the declared
//!    timeframe, not the previous event — the live feed publishes one
//!    `WorldEvent::Bar` per trade tick.

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use domain::money::{Price, Size};
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    StrategyDefinition,
};
use features::FeatureValue;
use strategy_runtime::history::{Unknown, Verdict};
use strategy_runtime::interpreter::{
    evaluate_signals, nodes_history_warmup_bars, nodes_need_history, SignalEvaluator,
};
use strategy_runtime::runtime::StrategyInstance;
use strategy_runtime::world::WorldEvent;

const INSTRUMENT: &str = "BTC-USDT";

fn nodes_for(expr: &str) -> Vec<Node> {
    vec![
        Node {
            id: "c1".into(),
            kind: NodeKind::Condition { expr: expr.into() },
        },
        Node {
            id: "s1".into(),
            kind: NodeKind::Signal {
                when: "c1".into(),
                emit: "long".into(),
            },
        },
    ]
}

fn def(expr: &str) -> StrategyDefinition {
    StrategyDefinition {
        exits: None,
        strategy_id: "cross_v1".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: domain::TrustTier::CentralizedExchange,
        inputs: vec![InputDeclaration {
            lane: "market.bars.1m".into(),
            instrument: "$bound_at_init".into(),
            features: vec![],
        }],
        nodes: nodes_for(expr),
        actions: vec![Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: domain::order::Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".into(),
                },
            },
        }],
        risk_overrides: domain::strategy_def::risk_overrides::RiskOverrides::default(),
        parameters: Default::default(),
        constraints: Vec::new(),
    }
}

fn bar_event(at: DateTime<Utc>, price: &str) -> WorldEvent {
    WorldEvent::Bar {
        instrument_id: INSTRUMENT.into(),
        timeframe: Timeframe::Minutes1,
        bar: BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str(price).unwrap(),
            Price::from_str(price).unwrap(),
            Price::from_str(price).unwrap(),
            Price::from_str(price).unwrap(),
            Size::from_str("1").unwrap(),
            1,
        ),
        available_time: at,
    }
}

fn feature_event(at: DateTime<Utc>, name: &str, value: f64) -> WorldEvent {
    WorldEvent::Feature {
        instrument_id: INSTRUMENT.into(),
        feature_value: FeatureValue::new(name, value, 1, at),
    }
}

fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// What one bar of the stream produced.
#[derive(Debug, Default, PartialEq)]
struct BarOutcome {
    orders: usize,
    unresolved: usize,
}

/// Feed one minute: the bar, then that minute's feature values, in the order
/// the live pipeline publishes them.
fn feed_minute(
    instance: &mut StrategyInstance,
    minute: i64,
    fast: f64,
    slow: f64,
) -> BarOutcome {
    let at = epoch() + Duration::minutes(minute);
    let mut out = BarOutcome::default();
    for event in [
        bar_event(at, "100"),
        feature_event(at, "fast", fast),
        feature_event(at, "slow", slow),
    ] {
        let result = instance.process_event(&event);
        out.orders += result.intents.len();
        out.unresolved += result.unresolved.len();
    }
    out
}

// ── the runtime instance ─────────────────────────────────────────────────────

/// The first bar has no bar before it, so the cross has no answer — and the
/// runtime says so rather than returning an empty intent list that reads
/// identically to "the strategy declined to trade".
#[test]
fn the_first_bar_is_reported_unresolved_not_silent() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );

    let first = feed_minute(&mut instance, 0, 9.0, 10.0);
    assert_eq!(first.orders, 0);
    assert!(
        first.unresolved > 0,
        "a bar with no previous bar must be reported, not counted as a no"
    );
}

/// The reason is carried, not just the fact: a caller can tell warm-up from a
/// broken definition.
#[test]
fn the_unresolved_report_names_the_signal_and_the_reason() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );
    let at = epoch();
    instance.process_event(&bar_event(at, "100"));
    instance.process_event(&feature_event(at, "fast", 9.0));
    let out = instance.process_event(&feature_event(at, "slow", 10.0));

    let u = out
        .unresolved
        .first()
        .expect("the cross could not be evaluated on the first bar");
    assert_eq!(u.signal, "long");
    assert_eq!(u.node, "c1");
    assert_eq!(u.reason, Unknown::HistoryNotReady { have: 0, need: 1 });
}

/// A cross places one order on the bar it happens, and the bars where the fast
/// line merely *remains* above the slow one place none.
#[test]
fn a_cross_enters_on_the_bar_it_happens() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );

    assert_eq!(feed_minute(&mut instance, 0, 9.0, 10.0).orders, 0);
    assert_eq!(feed_minute(&mut instance, 1, 9.5, 10.0).orders, 0);
    assert_eq!(
        feed_minute(&mut instance, 2, 10.5, 10.0).orders,
        1,
        "the crossing bar"
    );
    assert_eq!(feed_minute(&mut instance, 3, 11.0, 10.0).orders, 0);
    assert_eq!(feed_minute(&mut instance, 4, 12.0, 10.0).orders, 0);
}

/// The difference the level test cannot express, at the place it costs most.
///
/// A window that opens with the fast line already above the slow one has had no
/// crossing inside it. The level test reads the standing state as an event and
/// enters on the first bar; the cross says it does not know, then says no. Slice
/// the same data twelve ways and the level test invents twelve entries.
#[test]
fn a_window_opening_mid_trend_is_not_a_crossing() {
    let mut cross = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );
    let mut level = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("feature('fast') > feature('slow')"),
        epoch(),
    );

    let mut cross_orders = 0;
    let mut level_orders = 0;
    // Fast is above slow for the whole window and never crosses inside it.
    for minute in 0..4 {
        cross_orders += feed_minute(&mut cross, minute, 12.0, 10.0).orders;
        level_orders += feed_minute(&mut level, minute, 12.0, 10.0).orders;
    }

    assert_eq!(cross_orders, 0, "nothing crossed inside this window");
    assert_eq!(level_orders, 1, "the level test enters on the standing state");
}

/// The live feed publishes one `WorldEvent::Bar` per *trade tick*. A cross must
/// still be measured between bars of the declared timeframe, or a strategy that
/// declared minutes would be comparing two trades.
#[test]
fn ticks_inside_one_minute_are_one_bar() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );

    // Minute 0: sixty ticks, during which fast rises from below slow to above
    // it. Inside one bar that is not a crossing of anything — there is no
    // earlier bar to have been below.
    let mut orders = 0;
    for second in 0..60_i64 {
        let at = epoch() + Duration::seconds(second);
        let fast = if second < 30 { 9.0 } else { 11.0 };
        for event in [
            bar_event(at, "100"),
            feature_event(at, "fast", fast),
            feature_event(at, "slow", 10.0),
        ] {
            orders += instance.process_event(&event).intents.len();
        }
    }
    assert_eq!(
        orders, 0,
        "a within-bar move is not a cross; the previous bar is the unit"
    );

    // Minute 1 closes above, and minute 0 also closed above, so still no cross.
    assert_eq!(feed_minute(&mut instance, 1, 11.5, 10.0).orders, 0);
}

/// A cross declares the extra bar it needs, so a caller deriving data
/// requirements can add it to the indicator warm-up instead of losing the first
/// tradable bar of every window.
#[test]
fn a_cross_declares_the_extra_bar_it_needs() {
    let level = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("feature('fast') > feature('slow')"),
        epoch(),
    );
    assert_eq!(level.history_warmup_bars(), 0);

    let cross = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );
    assert_eq!(cross.history_warmup_bars(), 1);

    let rising = StrategyInstance::new("u", INSTRUMENT, def("rising(feature('fast'), 5)"), epoch());
    assert_eq!(rising.history_warmup_bars(), 5);
}

#[test]
fn an_instance_reports_when_its_history_is_warm() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("crossed_above(feature('fast'), feature('slow'))"),
        epoch(),
    );
    assert!(!instance.history_is_warm());
    feed_minute(&mut instance, 0, 9.0, 10.0);
    assert!(!instance.history_is_warm(), "still inside the first bar");
    feed_minute(&mut instance, 1, 9.5, 10.0);
    assert!(instance.history_is_warm());
}

/// A definition with no history function is untouched: no sites, no warm-up,
/// and nothing unresolved once its features arrive.
#[test]
fn a_level_only_definition_is_unaffected() {
    let mut instance = StrategyInstance::new(
        "u",
        INSTRUMENT,
        def("feature('fast') > feature('slow')"),
        epoch(),
    );
    assert_eq!(instance.history_warmup_bars(), 0);
    assert!(instance.history_is_warm(), "nothing to warm");

    let out = feed_minute(&mut instance, 0, 11.0, 10.0);
    assert_eq!(out.orders, 1);
}

/// `rising` over the same register mechanism, at a depth greater than one.
#[test]
fn rising_enters_only_after_a_monotone_run() {
    let mut instance =
        StrategyInstance::new("u", INSTRUMENT, def("rising(feature('fast'), 2)"), epoch());

    assert_eq!(feed_minute(&mut instance, 0, 1.0, 0.0).orders, 0);
    assert_eq!(feed_minute(&mut instance, 1, 2.0, 0.0).orders, 0);
    assert_eq!(
        feed_minute(&mut instance, 2, 3.0, 0.0).orders,
        1,
        "3 > 2 > 1 over the declared two bars"
    );
}

// ── the graph evaluator the replay driver needs ──────────────────────────────

fn features(pairs: &[(&str, f64)]) -> std::collections::HashMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

/// The stateless entry point cannot see a cross, and this is the test that says
/// so — so that nobody discovers it from a backtest that reported no trades.
#[test]
fn stateless_evaluate_signals_is_blind_to_a_cross() {
    let nodes = nodes_for("crossed_above(feature('fast'), feature('slow'))");
    let bars = std::collections::HashMap::new();

    // Fast is above slow and rose to get there, yet the stateless evaluator
    // emits nothing at all and has no way to report why.
    let fired = evaluate_signals(&nodes, &features(&[("fast", 11.0), ("slow", 10.0)]), &bars);
    assert!(fired.is_empty());

    // Which is exactly what `nodes_need_history` is for.
    assert!(nodes_need_history(&nodes));
    assert!(!nodes_need_history(&nodes_for(
        "feature('fast') > feature('slow')"
    )));
}

#[test]
fn the_graph_evaluator_separates_unknown_from_false() {
    let nodes = nodes_for("crossed_above(feature('fast'), feature('slow'))");
    let bars = std::collections::HashMap::new();
    let mut ev = SignalEvaluator::with_nodes(&nodes);
    assert_eq!(ev.warmup_bars(), 1);

    let first = ev.evaluate(&nodes, &features(&[("fast", 9.0), ("slow", 10.0)]), &bars, 0);
    assert!(first.fired.is_empty());
    assert_eq!(first.unresolved.len(), 1, "the first bar has no answer");
    assert!(!first.is_settled());

    let second = ev.evaluate(&nodes, &features(&[("fast", 9.5), ("slow", 10.0)]), &bars, 1);
    assert!(second.fired.is_empty());
    assert!(second.is_settled(), "a real no, and reported as settled");

    let third = ev.evaluate(
        &nodes,
        &features(&[("fast", 10.5), ("slow", 10.0)]),
        &bars,
        2,
    );
    assert_eq!(third.fired, ["long"]);

    let fourth = ev.evaluate(
        &nodes,
        &features(&[("fast", 11.0), ("slow", 10.0)]),
        &bars,
        3,
    );
    assert!(fourth.fired.is_empty(), "still above is not crossing again");
}

/// Re-slicing the same data must not quietly drop the entries at each window's
/// leading edge. The verdicts make that loss countable.
#[test]
fn every_window_reports_the_bars_it_could_not_answer() {
    let nodes = nodes_for("crossed_above(feature('fast'), feature('slow'))");
    let bars = std::collections::HashMap::new();

    // Three folds over the same shape, each starting fresh.
    let mut unresolved_total = 0;
    for _fold in 0..3 {
        let mut ev = SignalEvaluator::with_nodes(&nodes);
        for (i, fast) in [9.0, 9.5, 10.5].iter().enumerate() {
            let v = ev.evaluate(
                &nodes,
                &features(&[("fast", *fast), ("slow", 10.0)]),
                &bars,
                i as i64,
            );
            unresolved_total += v.unresolved.len();
        }
    }
    assert_eq!(
        unresolved_total, 3,
        "one unanswerable bar per fold, and every one of them reported"
    );
}

/// A bar clock that goes backwards is a broken clock. Reading a register
/// written by a later bar would be lookahead, so the history is dropped rather
/// than trusted.
#[test]
fn a_backwards_bar_invalidates_history_rather_than_reading_the_future() {
    let nodes = nodes_for("crossed_above(feature('fast'), feature('slow'))");
    let bars = std::collections::HashMap::new();
    let mut ev = SignalEvaluator::with_nodes(&nodes);

    ev.evaluate(&nodes, &features(&[("fast", 9.0), ("slow", 10.0)]), &bars, 5);
    ev.evaluate(&nodes, &features(&[("fast", 9.5), ("slow", 10.0)]), &bars, 6);

    let back = ev.evaluate(
        &nodes,
        &features(&[("fast", 10.5), ("slow", 10.0)]),
        &bars,
        4,
    );
    assert!(back.fired.is_empty());
    assert_eq!(
        back.unresolved.first().map(|u| u.reason),
        Some(Unknown::OutOfOrderBar)
    );
}

#[test]
fn warmup_is_derivable_from_the_node_graph_alone() {
    assert_eq!(
        nodes_history_warmup_bars(&nodes_for("feature('fast') > feature('slow')")),
        0
    );
    assert_eq!(
        nodes_history_warmup_bars(&nodes_for(
            "crossed_above(feature('fast'), feature('slow'))"
        )),
        1
    );
    assert_eq!(
        nodes_history_warmup_bars(&nodes_for("falling(feature('fast'), 9)")),
        9
    );
}

/// AGENT-011's headline example, in the shape `strategy-lang` compiles `and`
/// to — a parenthesised product of two comparisons.
#[test]
fn the_headline_example_compiles_and_runs() {
    let nodes = nodes_for(
        "(crossed_above(feature('ema_7'), feature('ema_21'))) * ((feature('rsi_14') < 60))",
    );
    let bars = std::collections::HashMap::new();
    let mut ev = SignalEvaluator::with_nodes(&nodes);

    let f = |ema7: f64, rsi: f64| features(&[("ema_7", ema7), ("ema_21", 10.0), ("rsi_14", rsi)]);

    assert!(ev.evaluate(&nodes, &f(9.0, 50.0), &bars, 0).unresolved.len() == 1);
    assert_eq!(ev.evaluate(&nodes, &f(11.0, 50.0), &bars, 1).fired, ["long"]);
    assert!(ev.evaluate(&nodes, &f(9.0, 50.0), &bars, 2).fired.is_empty());
    assert!(
        ev.evaluate(&nodes, &f(11.0, 70.0), &bars, 3)
            .fired
            .is_empty(),
        "crossed, but the rsi filter refused it"
    );
}

/// The verdict type refuses to let absent data become a `false` by accident.
#[test]
fn collapsing_unknown_is_something_a_caller_has_to_write() {
    let unknown = Verdict::Unknown(Unknown::FeatureNotReady);
    assert!(!unknown.is_true());
    assert!(unknown.is_unknown());
    assert_ne!(unknown, Verdict::False);
    // The only route from three values to two is named for what it costs.
    assert!(!unknown.unknown_as_false());
}

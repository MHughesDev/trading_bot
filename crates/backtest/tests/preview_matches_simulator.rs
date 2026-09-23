//! `preview_signals` agrees with the simulator (AGENT-011 §6; Set S task S-5.4).
//!
//! This is the acceptance, and it is the only property that makes the tool worth
//! having. A preview that disagreed with the backtest that follows it would be
//! worse than no preview: a model would adjust a strategy until the preview
//! looked right, spend a trial, get a different answer, and learn to ignore the
//! cheap tool in favour of the expensive one.
//!
//! What is compared is the **entry count and the entry bars**, because that is
//! what the preview claims. It does not claim fills, P&L or exits, and the
//! simulator's own tests cover those.
//!
//! # Two things this file had to learn about the simulator
//!
//! 1. `DetailedOutcome::trades` is a list of **closed round trips**, not entries
//!    — `Trade` carries both an `entry_time` and an `exit_time`. A definition
//!    with no exit policy opens a position that never closes and therefore
//!    reports *zero* trades however many times it fired. So the fixture here
//!    declares `MaxHoldingBars`, and the comparison is over entry timestamps
//!    with one position allowed to be still open when the window ends.
//! 2. `ema_N`'s warm-up is `5N` bars, not `N`. A fixture must be long enough
//!    that several crossings land *after* the slow feature starts producing
//!    values, or the preview is being asked about bars it correctly refuses to
//!    evaluate.

use backtest::preview::preview_signals;
use backtest::sim::{run_simulation_detailed, InstrumentPrecisions, SimulationInputs};
use backtest::store::LoadedBar;
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use nautilus_backtest::sdk::SimulationControl;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;

fn ts_ns(i: i64) -> i64 {
    i * 60_000_000_000
}

fn bar(i: i64, close: Decimal) -> LoadedBar {
    LoadedBar {
        ts_ns: ts_ns(i),
        open: close,
        high: close,
        low: close,
        close,
        volume: dec!(10),
        trade_count: 1,
        ..Default::default()
    }
}

/// A crossover, which is the shape that actually exercises the rising edge: the
/// condition goes true, stays true for a stretch, goes false, and comes back.
///
/// `exits` is spliced in so the same conditions can be run with and without a
/// self-closing policy — the entry logic is identical either way, and only the
/// simulator's *observable* output changes.
fn crossover_definition(exits: &str) -> StrategyDefinition {
    let json = format!(
        r#"{{
        "strategy_id": "ema_cross_preview_v1",
        "definition_version": "1.3",
        "asset_class": "crypto_spot_cex",
        "inputs": [
            {{ "lane": "market.bars.1m", "instrument": "$bound_at_init" }},
            {{ "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_3", "ema_10"] }}
        ],
        "nodes": [
            {{ "id": "n1", "type": "condition", "expr": "feature('ema_3') > feature('ema_10')" }},
            {{ "id": "n2", "type": "signal", "when": "n1", "emit": "long" }}
        ],
        "actions": [
            {{ "on_signal": "long", "type": "place_order",
              "order": {{ "side": "buy", "size_mode": "fixed", "size": "1" }} }}
        ],
        "exits": {exits}
    }}"#
    );
    serde_json::from_str(&json).expect("valid fixture definition")
}

/// `MaxHoldingBars` is the exit that makes entries countable: it closes on a bar
/// count rather than on a price level, so every entry produces exactly one
/// closed trade and nothing depends on the fixture's price path.
const TIMED_EXIT: &str =
    r#"{ "mode": "rules", "rules": [{ "rule": "max_holding_bars", "bars": 4 }] }"#;

/// Up and down repeatedly, long enough that the crossings land past `ema_10`'s
/// 50-bar warm-up.
///
/// The legs are short and steep on purpose: `ema_3` has to whip across `ema_10`
/// several times *after* bar 50, so that a preview counting "bars where the
/// condition held" instead of "bars where it became true" reports a number
/// several times too large and this fixture catches it.
fn zigzag() -> Vec<LoadedBar> {
    let mut bars = Vec::new();
    let mut i = 0i64;
    let mut price = 100i64;
    let mut push = |bars: &mut Vec<LoadedBar>, i: &mut i64, steps: usize, step: i64| {
        for _ in 0..steps {
            price += step;
            bars.push(bar(*i, Decimal::from(price)));
            *i += 1;
        }
    };
    // Seed the warm-up with a drift, so that by bar 50 both EMAs exist.
    push(&mut bars, &mut i, 60, 1);
    // Then eight reversals of 14 bars each: ~110 bars of live signal.
    for leg in 0..8 {
        let step = if leg % 2 == 0 { -3 } else { 3 };
        push(&mut bars, &mut i, 14, step);
    }
    bars
}

fn simulate(def: StrategyDefinition, bars: Vec<LoadedBar>) -> backtest::sim::DetailedOutcome {
    let features = backtest::requirements::derive_requirements(&def, Timeframe::Minutes1)
        .expect("requirements derive")
        .features;
    let inputs = SimulationInputs {
        definition: def,
        instrument_id: "BTC-USD".into(),
        venue_id: "coinbase_advanced".into(),
        asset_class: "crypto_spot_cex".into(),
        timeframe: Timeframe::Minutes1,
        quote_currency: "USD".into(),
        initial_balance: Decimal::from(1_000_000),
        precisions: Some(InstrumentPrecisions { price: 2, size: 4 }),
        sim_start_ns: 0,
        bars,
        features,
    };
    let control: Arc<SimulationControl> = SimulationControl::new();
    run_simulation_detailed(inputs, &control).expect("the simulation runs")
}

#[test]
fn the_preview_counts_the_entries_the_simulator_takes() {
    let bars = zigzag();
    let def = crossover_definition(TIMED_EXIT);
    let preview = preview_signals(&def, &bars, Timeframe::Minutes1).expect("the preview runs");
    let outcome = simulate(def, bars.clone());

    assert!(
        preview.entries > 1,
        "the zigzag fixture is meant to cross more than once; the preview found {} entries over \
         {} evaluated bars, so it is not exercising the rising edge at all",
        preview.entries,
        preview.bars_evaluated
    );

    // Every position the simulator opened, it opened on a bar the preview called
    // an entry. This is the comparison that matters: a count alone could agree
    // by coincidence while the two disagreed about *which* bars fired.
    let opened: Vec<i64> = outcome
        .trades
        .iter()
        .map(|t| t.entry_time.timestamp_nanos_opt().unwrap_or_default())
        .collect();
    let previewed: Vec<i64> = preview
        .sample_events
        .iter()
        .map(|e| e.at.timestamp_nanos_opt().unwrap_or_default())
        .collect();
    for at in &opened {
        assert!(
            previewed.contains(at),
            "the simulator opened a position at {at} and the preview did not call that bar an \
             entry. Previewed entries: {previewed:?}"
        );
    }

    // `sample_events` is capped, so a fixture that outgrew the cap would fail the
    // loop above for a reason that has nothing to do with the property. Said out
    // loud rather than left as a trap for whoever lengthens the fixture.
    assert_eq!(
        previewed.len(),
        preview.entries,
        "this comparison walks `sample_events`, which is capped; the fixture now takes {} entries \
         and only {} are reported, so shorten the fixture or compare another way",
        preview.entries,
        previewed.len()
    );

    // The counts agree up to a position still open when the window ends: the
    // last entry has nothing to close against, so it is an entry the preview
    // reports and the trade list cannot. Never the other way round — a trade the
    // preview did not predict is the failure this file exists to catch.
    assert!(
        opened.len() <= preview.entries,
        "the simulator opened {} positions and the preview predicted only {}",
        opened.len(),
        preview.entries
    );
    assert!(
        preview.entries - opened.len() <= 1,
        "the preview reports {} entries and the simulator opened {} positions. A preview that \
         disagrees with the backtest that follows it teaches the model to ignore it.",
        preview.entries,
        opened.len()
    );
}

/// The failure this is really guarding against, stated as its own test: counting
/// bars where the condition *held* rather than where it *became* true.
#[test]
fn the_preview_counts_edges_not_bars_the_condition_was_true_on() {
    let bars = zigzag();
    let preview = preview_signals(
        &crossover_definition(TIMED_EXIT),
        &bars,
        Timeframe::Minutes1,
    )
    .expect("the preview runs");
    let long = preview
        .signals
        .iter()
        .find(|s| s.signal == "long")
        .expect("the long signal is reported");

    assert!(
        long.bars_true > long.fires,
        "on this fixture the condition holds for stretches, so `bars_true` ({}) must exceed \
         `fires` ({}). Equal counts mean the rising edge is not being applied.",
        long.bars_true,
        long.fires
    );
}

/// A strategy that cannot fire says which of the reasons it was, rather than
/// reporting a bare zero that sends the author to change the wrong thing.
#[test]
fn a_window_shorter_than_the_warmup_says_so() {
    // 40 bars against a 100-bar EMA: the feature never produces a value.
    let bars: Vec<LoadedBar> = (0..40).map(|i| bar(i, Decimal::from(100 + i))).collect();
    let json = r#"{
        "strategy_id": "slow_ema_v1",
        "definition_version": "1.0",
        "asset_class": "crypto_spot_cex",
        "inputs": [
            { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_100"] }
        ],
        "nodes": [
            { "id": "n1", "type": "condition", "expr": "feature('ema_100') > 0.0" },
            { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
        ],
        "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }
        ]
    }"#;
    let def: StrategyDefinition = serde_json::from_str(json).expect("valid fixture");

    let preview = preview_signals(&def, &bars, Timeframe::Minutes1).expect("the preview runs");
    assert_eq!(preview.entries, 0);
    let why = preview
        .never_fires_because
        .expect("a strategy that never fires says why");
    assert!(
        why.contains("warm-up") || why.contains("warm"),
        "the reason should name the warm-up, since that is what is wrong here: {why}"
    );
}

/// A signal with no action behind it is not an entry, and the preview says which
/// block to go and fix.
///
/// The simulator places an order only where an action's `on_signal` matches the
/// signal that rose (`sim.rs:1109`). A preview counting every rising edge would
/// promise entries for a definition that cannot take one — the exact
/// disagreement this file exists to prevent, and the one that is invisible on a
/// fixture where every signal happens to be traded.
#[test]
fn a_signal_no_action_names_is_not_an_entry() {
    let bars = zigzag();
    let json = r#"{
        "strategy_id": "unwired_v1",
        "definition_version": "1.0",
        "asset_class": "crypto_spot_cex",
        "inputs": [
            { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_3", "ema_10"] }
        ],
        "nodes": [
            { "id": "n1", "type": "condition", "expr": "feature('ema_3') > feature('ema_10')" },
            { "id": "n2", "type": "signal", "when": "n1", "emit": "regime_risk_on" }
        ],
        "actions": []
    }"#;
    let def: StrategyDefinition = serde_json::from_str(json).expect("valid fixture");

    let preview = preview_signals(&def, &bars, Timeframe::Minutes1).expect("the preview runs");
    let flag = preview
        .signals
        .iter()
        .find(|s| s.signal == "regime_risk_on")
        .expect("the signal is still reported — a caller wants to see the flag firing");

    assert!(
        flag.fires > 1,
        "the fixture's condition rises several times; the signal stats should say so ({} fires)",
        flag.fires
    );
    assert_eq!(
        preview.entries, 0,
        "no action names `regime_risk_on`, so the simulator would place no order and the preview \
         must not claim {} entries",
        preview.entries
    );
    let why = preview
        .never_fires_because
        .expect("a strategy that takes no entry says why");
    assert!(
        why.contains("no action names it"),
        "the reason should send the author to `actions`, which is the only block that is wrong \
         here: {why}"
    );
}

/// Each diagnosis carries a stable code, not only prose.
///
/// The prose says which of the reasons it was, for a person. The code is what a
/// retry policy, a renderer and `strategy_validator::teach` can act on — and
/// AGENT-011 §5 names two of these (`warmup_exceeds_window`, `never_fires`) as
/// error cases that must return a fix. Prose cannot be keyed on: re-deriving a
/// code by matching the message is the "dedup key built from a display string"
/// mistake this repository has already paid for once.
#[test]
fn every_diagnosis_carries_a_code_a_caller_can_branch_on() {
    // Warm-up: 40 bars against a 100-bar EMA.
    let short: Vec<LoadedBar> = (0..40).map(|i| bar(i, Decimal::from(100 + i))).collect();
    let slow = r#"{
        "strategy_id": "slow_v1", "definition_version": "1.0", "asset_class": "crypto_spot_cex",
        "inputs": [{ "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                   { "lane": "features.technical", "instrument": "$bound_at_init",
                     "features": ["ema_100"] }],
        "nodes": [{ "id": "n1", "type": "condition", "expr": "feature('ema_100') > 0.0" },
                  { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }],
        "actions": [{ "on_signal": "long", "type": "place_order",
                      "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }]
    }"#;
    let def: StrategyDefinition = serde_json::from_str(slow).expect("fixture");
    let p = preview_signals(&def, &short, Timeframe::Minutes1).expect("the preview runs");
    assert_eq!(
        p.never_fires_code,
        Some("warmup_exceeds_window"),
        "a window shorter than the warm-up is AGENT-011 §5's `warmup_exceeds_window`, and the \
         prose was: {:?}",
        p.never_fires_because
    );

    // A signal nothing trades.
    let bars = zigzag();
    let unwired = r#"{
        "strategy_id": "unwired_v2", "definition_version": "1.0", "asset_class": "crypto_spot_cex",
        "inputs": [{ "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                   { "lane": "features.technical", "instrument": "$bound_at_init",
                     "features": ["ema_3", "ema_10"] }],
        "nodes": [{ "id": "n1", "type": "condition", "expr": "feature('ema_3') > feature('ema_10')" },
                  { "id": "n2", "type": "signal", "when": "n1", "emit": "flag" }],
        "actions": []
    }"#;
    let def: StrategyDefinition = serde_json::from_str(unwired).expect("fixture");
    let p = preview_signals(&def, &bars, Timeframe::Minutes1).expect("the preview runs");
    assert_eq!(
        p.never_fires_code,
        Some("signal_has_no_action"),
        "prose was: {:?}",
        p.never_fires_because
    );

    // A condition nothing satisfies.
    let never = r#"{
        "strategy_id": "never_v1", "definition_version": "1.0", "asset_class": "crypto_spot_cex",
        "inputs": [{ "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                   { "lane": "features.technical", "instrument": "$bound_at_init",
                     "features": ["ema_3"] }],
        "nodes": [{ "id": "n1", "type": "condition", "expr": "feature('ema_3') > 1000000.0" },
                  { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }],
        "actions": [{ "on_signal": "long", "type": "place_order",
                      "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }]
    }"#;
    let def: StrategyDefinition = serde_json::from_str(never).expect("fixture");
    let p = preview_signals(&def, &bars, Timeframe::Minutes1).expect("the preview runs");
    assert_eq!(
        p.never_fires_code,
        Some("never_fires"),
        "prose was: {:?}",
        p.never_fires_because
    );

    // And a strategy that does fire carries no diagnosis at all.
    let firing = crossover_definition(TIMED_EXIT);
    let p = preview_signals(&firing, &bars, Timeframe::Minutes1).expect("the preview runs");
    assert_eq!(p.never_fires_code, None);
    assert_eq!(p.never_fires_because, None);
}

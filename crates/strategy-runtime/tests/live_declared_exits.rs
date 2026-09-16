//! The **live** runtime honours the declared exit policy (ADR-P6-14, SC-3).
//!
//! Track 15 made the backtest simulator drive `ExitTracker` and said plainly
//! that the live runtime still did not. That left the platform in the one state
//! worse than the defect it fixed: a declared stop-loss bounded a backtest and
//! not the deployment the backtest was used to justify. These are the live half
//! of `backtest/tests/declared_exits.rs`, deliberately in the same shape and
//! with the same hand-computed expectations, because the point is that the two
//! paths agree.
//!
//! Every expectation is stated in the assertion message and derived from the
//! bars in the test, never from running the code and recording what it said.

use chrono::{DateTime, Duration, Utc};
use domain::money::{Price, Size};
use domain::order::Side;
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::exits::{ExitPolicy, ExitRule, PositionSide};
use domain::strategy_def::StrategyDefinition;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_runtime::exits::{Bar as ExitBar, ExitTracker};
use strategy_runtime::runtime::StrategyInstance;
use strategy_runtime::world::WorldEvent;

/// Minute-aligned, so bar `n` is unambiguously its own minute.
fn ts(n: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 - 20 + n * 60, 0).expect("valid timestamp")
}

/// A definition that goes long one unit on its first bar and carries `exits`.
///
/// `bar('close') > 0.0` is true on every bar, so the rising-edge semantics the
/// runtime gives actions open exactly one position, on bar 0, and never
/// re-enter. That is the same fixture shape the simulator's suite uses.
fn long_with_exits(exits: Option<ExitPolicy>) -> StrategyDefinition {
    let version = if exits.is_some() { "1.3" } else { "1.0" };
    let exits_block = exits
        .map(|e| format!(r#""exits": {},"#, serde_json::to_string(&e).unwrap()))
        .unwrap_or_default();
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

fn rules(rules: Vec<ExitRule>) -> Option<ExitPolicy> {
    Some(ExitPolicy::Rules { rules })
}

/// One whole bar as a single event — the shape a bar feed delivers, and the
/// shape the simulator sees.
fn bar_event(n: i64, high: Decimal, low: Decimal, close: Decimal) -> WorldEvent {
    WorldEvent::Bar {
        instrument_id: "BTC-USD".into(),
        timeframe: Timeframe::Minutes1,
        bar: BarPayload::new(
            Timeframe::Minutes1,
            Price::from_decimal(close),
            Price::from_decimal(high),
            Price::from_decimal(low),
            Price::from_decimal(close),
            Size::from_decimal(dec!(10)),
            n as u64,
        ),
        available_time: ts(n),
    }
}

/// A bar where nothing is meant to happen.
fn flat_bar(n: i64, p: Decimal) -> WorldEvent {
    bar_event(n, p, p, p)
}

/// One trade tick, as the hot path publishes it: the traded price as all four
/// of O/H/L/C, `seconds` into minute `n`.
fn tick_event(n: i64, seconds: i64, price: Decimal) -> WorldEvent {
    WorldEvent::Bar {
        instrument_id: "BTC-USD".into(),
        timeframe: Timeframe::Minutes1,
        bar: BarPayload::new(
            Timeframe::Minutes1,
            Price::from_decimal(price),
            Price::from_decimal(price),
            Price::from_decimal(price),
            Price::from_decimal(price),
            Size::from_decimal(Decimal::ONE),
            n as u64,
        ),
        available_time: ts(n) + Duration::seconds(seconds),
    }
}

fn instance(def: StrategyDefinition) -> StrategyInstance {
    StrategyInstance::new("user1", "BTC-USD", def, ts(0))
}

/// Drive `events` and return, per event, the intents it produced.
fn drive(inst: &mut StrategyInstance, events: &[WorldEvent]) -> Vec<Vec<domain::OrderIntent>> {
    // `.intents`: `process_event` returns `SizedIntents`, which carries the
    // sizing errors alongside the orders. These tests are about *which* orders
    // a declared exit produces, and a sizing error is asserted on where it is
    // the subject; dropping them silently here would not be.
    events
        .iter()
        .map(|e| {
            let out = inst.process_event(e);
            assert!(out.errors.is_empty(), "unexpected sizing error: {:?}", out.errors);
            out.intents
        })
        .collect()
}

/// The index of the first event that produced a flattening sell, if any.
fn exit_on(per_event: &[Vec<domain::OrderIntent>]) -> Option<usize> {
    per_event
        .iter()
        .position(|intents| intents.iter().any(|i| i.side == Side::Sell))
}

// ── the three exit kinds, each at the bar the arithmetic says ────────────────

/// Entry at 100 on bar 0's close; a 2% stop rests at 98. Bar 1 is quiet. Bar 2
/// trades down to 97 and closes back up at 100.80.
///
/// It must fire on bar 2 — not bar 1, and not never — and the level it reports
/// must be 98, the price the order rested at. 100.80 is what a market order
/// sent at the close would have paid, and booking that would report a gain on
/// an order that lost 2%.
#[test]
fn a_declared_stop_loss_fires_live_on_the_breaching_bar() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::StopLoss {
        fraction: dec!(0.02),
    }])));

    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            flat_bar(1, dec!(100)),
            bar_event(2, dec!(100.80), dec!(97), dec!(100.80)),
        ],
    );

    assert_eq!(
        per_event[0].len(),
        1,
        "bar 0 opens the position and nothing else"
    );
    assert_eq!(per_event[0][0].side, Side::Buy);
    assert!(
        per_event[1].is_empty(),
        "bar 1 traded at 100 throughout; a 98 stop has no business firing"
    );

    assert_eq!(exit_on(&per_event), Some(2), "the stop fires on bar 2");
    let exit = &per_event[2][0];
    assert_eq!(exit.side, Side::Sell, "flattening a long is a sale");
    assert_eq!(exit.size.inner(), Decimal::ONE, "the whole position");

    let fired = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(fired.rule, "stop_loss");
    assert_eq!(
        fired.price,
        dec!(98),
        "the level the order rested at: 100 × (1 − 0.02). Not 100.80, which is \
         what a market order at the close would have paid"
    );
    assert_eq!(fired.bars_held, 2, "entry on bar 0's close, exit on bar 2");
}

/// Entry at 100; a 5% target rests at 105. Bar 1 is quiet. Bar 2 trades up to
/// 120 and closes at 118.
///
/// The level is 105, not 120 and not 118: a resting sell at 105 is filled the
/// moment the bar trades through it, and claiming the extreme is a windfall
/// nobody earned.
#[test]
fn a_declared_take_profit_fires_live_at_its_level() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::TakeProfit {
        fraction: dec!(0.05),
    }])));

    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            flat_bar(1, dec!(100)),
            bar_event(2, dec!(120), dec!(100), dec!(118)),
        ],
    );

    assert!(per_event[1].is_empty(), "bar 1 never reached 105");
    assert_eq!(exit_on(&per_event), Some(2));
    assert_eq!(per_event[2][0].side, Side::Sell);

    let fired = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(fired.rule, "take_profit");
    assert_eq!(
        fired.price,
        dec!(105),
        "100 × 1.05 — no windfall from the bar's high of 120"
    );
    assert_eq!(fired.bars_held, 2);
}

/// `max_holding_bars: 2` on a position opened at the close of bar 0.
///
/// Bar 1 is holding bar one and bar 2 is holding bar two, so it closes on bar
/// 2. Firing on bar 3 would be a strategy holding longer than it declared, and
/// firing on bar 1 would be shorter — both make the declared holding period,
/// which is what the tradeable-frontier check reads, a fiction.
#[test]
fn a_declared_time_exit_fires_live_on_the_declared_bar() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::MaxHoldingBars {
        bars: 2,
    }])));

    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            flat_bar(1, dec!(100)),
            flat_bar(2, dec!(100)),
            flat_bar(3, dec!(100)),
        ],
    );

    assert!(per_event[1].is_empty(), "one bar held is not two");
    assert_eq!(exit_on(&per_event), Some(2), "two bars held closes on bar 2");
    assert!(
        per_event[3].is_empty(),
        "the position is already flat; nothing may be sent again"
    );

    let fired = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(fired.rule, "max_holding_bars");
    assert_eq!(fired.bars_held, 2);
}

// ── the intrabar rule holds live too ─────────────────────────────────────────

/// A bar that breached both the 98 stop and the 105 target resolves to the
/// stop. OHLCV cannot say which came first, and the live runtime has no more
/// right to guess in the strategy's favour than the simulator does.
#[test]
fn a_bar_that_breached_both_books_the_stop_live_and_never_the_target() {
    let mut inst = instance(long_with_exits(rules(vec![
        ExitRule::TakeProfit { fraction: dec!(0.05) },
        ExitRule::StopLoss { fraction: dec!(0.02) },
    ])));

    let per_event = drive(
        &mut inst,
        &[flat_bar(0, dec!(100)), bar_event(1, dec!(106), dec!(97), dec!(104))],
    );

    assert_eq!(exit_on(&per_event), Some(1));
    let fired = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(fired.rule, "stop_loss", "the adverse extreme is probed first");
    assert_eq!(fired.price, dec!(98));
}

/// The entry bar's extremes happened before the position existed. A bar that
/// traded to 90 and closed at 100 must not stop out a position opened at that
/// close, however tempting the low looks.
#[test]
fn the_entry_bar_cannot_stop_out_a_position_opened_at_its_close() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::StopLoss {
        fraction: dec!(0.02),
    }])));

    let per_event = drive(
        &mut inst,
        &[
            bar_event(0, dec!(100), dec!(90), dec!(100)),
            flat_bar(1, dec!(100)),
        ],
    );

    assert_eq!(per_event[0].len(), 1, "the entry, and nothing else");
    assert_eq!(per_event[0][0].side, Side::Buy);
    assert!(
        exit_on(&per_event).is_none(),
        "the 90 was traded before the position existed"
    );
}

// ── SC-3: the same sequence, the same answer, on both paths ──────────────────

/// The live runtime and the simulator's own driving of `ExitTracker` must fire
/// the time exit on the same bar.
///
/// This is the one that would have caught the bug. The simulator opens through
/// `opened_at_bar_close`, which adds the holding bar its entry bar never
/// supplies; the live runtime opens through `opened` and lets the roll out of
/// the entry bar supply it instead. Two routes to one clock, and the only way
/// to know they arrive at the same place is to run both.
#[test]
fn live_and_replay_fire_the_time_exit_on_the_same_bar() {
    for declared in 1..=4u32 {
        let bars: Vec<(Decimal, Decimal, Decimal)> =
            (0..8).map(|_| (dec!(101), dec!(99), dec!(100))).collect();

        // Replay: the simulator's convention, one whole bar per call, the entry
        // bar consumed before the tracker ever sees one.
        let mut replay = ExitTracker::new(ExitPolicy::Rules {
            rules: vec![ExitRule::MaxHoldingBars { bars: declared }],
        });
        replay.opened_at_bar_close(PositionSide::Long, dec!(100));
        let replay_bar = bars
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(n, (h, l, c))| {
                let probe = ExitBar { ts: ts(n as i64), high: *h, low: *l, close: *c };
                replay.on_bar(&probe, &[]).map(|_| n)
            })
            .expect("replay must fire");

        // Live: the same bars through the whole runtime.
        let mut inst = instance(long_with_exits(rules(vec![ExitRule::MaxHoldingBars {
            bars: declared,
        }])));
        let events: Vec<WorldEvent> = bars
            .iter()
            .enumerate()
            .map(|(n, (h, l, c))| bar_event(n as i64, *h, *l, *c))
            .collect();
        let live_bar = exit_on(&drive(&mut inst, &events)).expect("live must fire");

        assert_eq!(
            live_bar, replay_bar,
            "max_holding_bars: {declared} — live fired on bar {live_bar} and replay on \
             bar {replay_bar}. A holding period that differs between the two makes every \
             backtest a statement about a different strategy"
        );
        assert_eq!(
            inst.last_declared_exit().map(|e| e.bars_held),
            Some(declared),
            "and both held exactly what was declared"
        );
    }
}

/// The tick feed and the bar feed must agree.
///
/// The hot path publishes one `WorldEvent::Bar` per *trade*, each carrying that
/// tick's price as all four of O/H/L/C. Counting those as bars would make
/// `max_holding_bars: 2` mean two trades live and two minutes in replay, which
/// on a liquid instrument is a difference of three orders of magnitude.
#[test]
fn a_tick_feed_counts_the_same_bars_as_a_bar_feed() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::MaxHoldingBars {
        bars: 2,
    }])));

    // Minute 0 opens the position on its first tick; twenty more ticks follow
    // inside the same minute, then twenty in minute 1, then minute 2.
    let mut events = Vec::new();
    for minute in 0..3 {
        for t in 0..20 {
            events.push(tick_event(minute, t * 3, dec!(100)));
        }
    }
    let per_event = drive(&mut inst, &events);

    let entries = per_event
        .iter()
        .flatten()
        .filter(|i| i.side == Side::Buy)
        .count();
    assert_eq!(entries, 1, "one rising edge is one entry, not sixty");

    let fired = exit_on(&per_event).expect("the time exit must fire");
    assert_eq!(
        fired / 20,
        2,
        "it must fire during minute 2 — event {fired} is in minute {}. Sixty ticks are \
         three minutes, not sixty bars",
        fired / 20
    );
    assert_eq!(
        inst.last_declared_exit().map(|e| e.bars_held),
        Some(2),
        "two declared bars held"
    );
}

/// A stop fires on the tick that breaches it, not at the end of the minute.
///
/// Live is finer-grained than replay and the difference is allowed to run in
/// exactly one direction: earlier. A bound on loss that waits for the bar to
/// close is not the bound the strategy declared.
#[test]
fn a_live_stop_fires_on_the_tick_that_breaches_it() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::StopLoss {
        fraction: dec!(0.02),
    }])));

    let mut events = vec![tick_event(0, 0, dec!(100))];
    // Minute 1: drifts down, breaches 98 at the sixth tick, recovers.
    for (t, p) in [
        (0, dec!(100)),
        (10, dec!(99.5)),
        (20, dec!(99)),
        (30, dec!(98.5)),
        (40, dec!(98.2)),
        (50, dec!(97)),
    ] {
        events.push(tick_event(1, t, p));
    }
    events.push(tick_event(2, 0, dec!(104)));

    let per_event = drive(&mut inst, &events);
    let fired = exit_on(&per_event).expect("the stop must fire");
    assert_eq!(
        fired, 6,
        "the tick that traded 97 is event 6; waiting for minute 1 to close would have \
         been event 7, at 104, with the loss unbounded in between"
    );
    let exit = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(exit.price, dec!(98), "the level, not the 97 it traded to");
}

// ── the two ways of declaring no exit, kept apart ────────────────────────────

/// A pre-1.3 definition carries no policy and nothing closes it here.
#[test]
fn a_definition_with_no_declared_exits_is_untouched_live() {
    let mut inst = instance(long_with_exits(None));
    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            bar_event(1, dec!(100), dec!(50), dec!(60)),
            flat_bar(2, dec!(60)),
        ],
    );
    assert!(
        exit_on(&per_event).is_none(),
        "a 40% drawdown closes nothing a definition never declared"
    );
    assert!(inst.last_declared_exit().is_none());
}

/// `RiskLayerOnly` is a decision, not an absence — and the decision is that
/// this layer closes nothing.
#[test]
fn risk_layer_only_closes_nothing_live() {
    let mut inst = instance(long_with_exits(Some(ExitPolicy::RiskLayerOnly)));
    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            bar_event(1, dec!(100), dec!(50), dec!(60)),
        ],
    );
    assert!(exit_on(&per_event).is_none());
}

// ── the venue's word outranks the instance's assumption ──────────────────────

/// An entry the venue never filled must not leave a phantom position that a
/// stop then "closes" — which would send a sell for something not held.
#[test]
fn a_position_the_venue_reports_flat_releases_the_tracker() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::StopLoss {
        fraction: dec!(0.02),
    }])));

    inst.process_event(&flat_bar(0, dec!(100)));
    assert_eq!(inst.exit_bars_held(), Some(0), "the entry was assumed");

    // The risk gate rejected it, or the broker did.
    inst.process_event(&WorldEvent::PositionUpdate {
        instrument_id: "BTC-USD".into(),
        quantity: Decimal::ZERO,
    });
    assert_eq!(inst.exit_bars_held(), None, "the assumption is released");

    let after = inst.process_event(&bar_event(1, dec!(100), dec!(90), dec!(95)));
    assert!(
        after.intents.iter().all(|i| i.side != Side::Sell),
        "nothing may be sold for a position that was never opened"
    );
}

// ── the policy's own parameters are what is driven ───────────────────────────

/// A trailing stop ratchets on the running high across bars, live as in
/// replay. Entry 100; bar 1 reaches 110; the 3% band then sits at 106.70 and
/// bar 2's retrace to 105 breaches it.
#[test]
fn a_trailing_stop_ratchets_live_across_bars() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::TrailingStop {
        fraction: dec!(0.03),
    }])));

    let per_event = drive(
        &mut inst,
        &[
            flat_bar(0, dec!(100)),
            bar_event(1, dec!(110), dec!(108.5), dec!(109)),
            bar_event(2, dec!(109), dec!(105), dec!(106)),
        ],
    );

    assert!(per_event[1].is_empty(), "108.5 is inside a 3% band from 110");
    assert_eq!(exit_on(&per_event), Some(2));
    let fired = inst.last_declared_exit().expect("an exit was recorded");
    assert_eq!(fired.rule, "trailing_stop");
    assert_eq!(
        fired.price,
        dec!(110) * (Decimal::ONE - dec!(0.03)),
        "110 × 0.97 = 106.70, measured from the running high and not from entry"
    );
}

/// The `on_signal` exit reads the signals emitted on the bar, which is a
/// different question from whether they just became true — the same split the
/// simulator makes between the full emitted set and the rising edge.
#[test]
fn the_signal_exit_reads_the_emitted_set_not_the_rising_edge() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::OnSignal {
        emit: "long".into(),
    }])));

    let per_event = drive(&mut inst, &[flat_bar(0, dec!(100)), flat_bar(1, dec!(100))]);

    assert_eq!(per_event[0].len(), 1, "bar 0 is the entry");
    assert_eq!(
        exit_on(&per_event),
        Some(1),
        "`long` is still emitted on bar 1 — still true is what an on_signal exit asks"
    );
    assert_eq!(
        inst.last_declared_exit().map(|e| e.rule.clone()),
        Some("on_signal".to_owned())
    );
}

/// Nothing here may reach for an OS clock: every timestamp comes from the
/// event's `available_time`, so the same events replay to the same exits.
#[test]
fn the_exit_timestamp_comes_from_the_event_not_the_wall_clock() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::MaxHoldingBars {
        bars: 1,
    }])));
    drive(&mut inst, &[flat_bar(0, dec!(100)), flat_bar(1, dec!(100))]);
    assert_eq!(
        inst.last_declared_exit().map(|e| e.ts),
        Some(ts(1)),
        "the bar's available_time, which a replay reproduces exactly"
    );
}

/// The whole point, stated once as a property: for a bar-per-event feed, the
/// live runtime and the simulator's tracker agree on every bar of a walk that
/// includes a stop, a target and a time limit at once.
#[test]
fn live_and_replay_agree_bar_for_bar_on_a_mixed_policy() {
    let policy = || {
        vec![
            ExitRule::StopLoss { fraction: dec!(0.03) },
            ExitRule::TakeProfit { fraction: dec!(0.04) },
            ExitRule::MaxHoldingBars { bars: 5 },
        ]
    };
    // A walk that reaches neither level until bar 4, where it breaches both.
    let walk: Vec<(Decimal, Decimal, Decimal)> = vec![
        (dec!(100), dec!(100), dec!(100)),
        (dec!(101), dec!(99), dec!(100)),
        (dec!(102), dec!(98.5), dec!(101)),
        (dec!(102), dec!(98), dec!(99)),
        (dec!(105), dec!(96), dec!(103)),
        (dec!(100), dec!(100), dec!(100)),
    ];

    let mut replay = ExitTracker::new(ExitPolicy::Rules { rules: policy() });
    replay.opened_at_bar_close(PositionSide::Long, dec!(100));
    let replay_fired = walk.iter().enumerate().skip(1).find_map(|(n, (h, l, c))| {
        let probe = ExitBar { ts: ts(n as i64), high: *h, low: *l, close: *c };
        replay.on_bar(&probe, &[]).map(|t| (n, t))
    });

    let mut inst = instance(long_with_exits(rules(policy())));
    let events: Vec<WorldEvent> = walk
        .iter()
        .enumerate()
        .map(|(n, (h, l, c))| bar_event(n as i64, *h, *l, *c))
        .collect();
    let live_bar = exit_on(&drive(&mut inst, &events));

    let (replay_bar, replay_triggered) = replay_fired.expect("replay must fire");
    assert_eq!(live_bar, Some(replay_bar), "same bar");
    let live_triggered = inst.last_declared_exit().expect("live must fire");
    assert_eq!(live_triggered.rule, replay_triggered.rule, "same rule");
    assert_eq!(live_triggered.price, replay_triggered.price, "same level");
    assert_eq!(
        live_triggered.bars_held, replay_triggered.bars_held,
        "same holding period"
    );
    assert_eq!(
        replay_triggered.rule, "stop_loss",
        "bar 4 breached both; the adverse one wins on both paths"
    );
}

/// Guards the fixture itself: a definition with no signal emits nothing, so a
/// passing exit test cannot be passing because the entry never happened.
#[test]
fn the_fixture_really_does_open_a_position() {
    let mut inst = instance(long_with_exits(rules(vec![ExitRule::MaxHoldingBars {
        bars: 99,
    }])));
    let per_event = drive(&mut inst, &[flat_bar(0, dec!(100))]);
    assert_eq!(per_event[0].len(), 1);
    assert_eq!(per_event[0][0].side, Side::Buy);
    assert_eq!(per_event[0][0].size.inner(), Decimal::ONE);
    assert_eq!(
        inst.exit_bars_held(),
        Some(0),
        "the tracker is holding it, which is what makes the other tests mean anything"
    );
}

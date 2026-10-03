//! Set U U-6.3: running a selection now or as of a past instant.
//!
//! Acceptance (the pure half): a run as of a past date uses that date's
//! membership and data only. The other half — a person and the agent get the
//! same list — is `crates/api/tests/selections_route.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use backtest::runner;
use backtest::selection_run::{evaluate_over, evaluation_instant};
use backtest::store::LoadedBar;
use chrono::{TimeZone, Utc};
use dataplane::quality::QualityFlags;
use dataplane::universe::MembershipEvent;
use dataplane::InstrumentKey;
use domain::strategy_graph::selection::ExclusionReason;
use domain::strategy_graph::StrategyGraph;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::{Membership, StdRegistries};

const DAY: i64 = 86_400_000_000_000;

fn t0_ns() -> i64 {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap()
}

fn daily(closes: &[f64]) -> Vec<LoadedBar> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let open_ns = t0_ns() + i as i64 * DAY;
            let p = Decimal::from_f64_retain(c).unwrap();
            LoadedBar {
                ts_ns: open_ns + DAY,
                open_ns,
                knowledge_ns: open_ns + DAY,
                open: p,
                high: p,
                low: p,
                close: p,
                volume: dec!(1),
                trade_count: 1,
                quality_flags: QualityFlags::default(),
            }
        })
        .collect()
}

fn plan() -> Arc<strategy_engine::Plan> {
    let g: StrategyGraph = serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "momentum",
      "asset_class": "crypto_spot_cex", "timeframe": "1d",
      "universe": {"kind": "named", "name": "u"},
      "nodes": [
        {"id":"mom", "op":"change", "args":{"x":"close","n":3}},
        {"id":"z",   "op":"cs_rank","args":{"x":"mom"}}
      ],
      "select": {"score":"mom", "top_n": 1, "attributes": {"rank":"z"}}
    }))
    .unwrap();
    Arc::new(
        runner::compile_for_run(&g, &backtest::run::ParamMap::new(), &StdRegistries::new())
            .unwrap(),
    )
}

/// A rises steadily. B joins on day 6 and rises far faster from then on.
fn bars() -> BTreeMap<String, Vec<Vec<LoadedBar>>> {
    let a: Vec<f64> = (0..12).map(|i| 100.0 + f64::from(i)).collect();
    let b: Vec<f64> = (0..12)
        .map(|i| {
            if i < 6 {
                50.0
            } else {
                50.0 + 20.0 * f64::from(i - 5)
            }
        })
        .collect();
    BTreeMap::from([
        ("A".to_string(), vec![daily(&a)]),
        ("B".to_string(), vec![daily(&b)]),
    ])
}

fn membership() -> Membership {
    let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    Membership::logged(
        vec![
            MembershipEvent::added("u", InstrumentKey(1), start, "seed"),
            MembershipEvent::added(
                "u",
                InstrumentKey(2),
                start + chrono::Duration::days(6),
                "listed",
            ),
        ],
        BTreeMap::from([(1, "A".to_string()), (2, "B".to_string())]),
    )
    .unwrap()
}

#[test]
fn as_of_a_past_date_the_run_sees_that_dates_membership_and_data_only() {
    let p = plan();
    let early =
        evaluate_over(&p, &membership(), &bars(), t0_ns() + 5 * DAY, &mut NoModels).unwrap();
    assert_eq!(
        early.selection.universe_size, 1,
        "B is not a member on day 5"
    );
    assert_eq!(early.selection.instruments(), ["A"]);
    assert!(!early.values.contains_key("B"));

    let late = evaluate_over(
        &p,
        &membership(),
        &bars(),
        t0_ns() + 10 * DAY,
        &mut NoModels,
    )
    .unwrap();
    assert_eq!(late.selection.universe_size, 2);
    assert_eq!(late.selection.instruments(), ["B"], "by day 10 B leads");

    // The day-5 answer is the same whatever comes after: bars closing later
    // are not read.
    let mut truncated = bars();
    for inputs in truncated.values_mut() {
        inputs[0].retain(|b| b.ts_ns <= t0_ns() + 5 * DAY);
    }
    let again = evaluate_over(
        &p,
        &membership(),
        &truncated,
        t0_ns() + 5 * DAY,
        &mut NoModels,
    )
    .unwrap();
    assert_eq!(again.digest(), early.digest());
}

#[test]
fn a_bar_recorded_after_the_instant_is_not_read() {
    let p = plan();
    let at = t0_ns() + 8 * DAY;
    let honest = evaluate_over(&p, &membership(), &bars(), at, &mut NoModels).unwrap();
    // Day 8's bar for A, revised and only recorded on day 9: as of day 8 it does
    // not exist yet, so A has fewer bars and its change(3) is from earlier data.
    let mut late = bars();
    let a = &mut late.get_mut("A").unwrap()[0];
    let day8 = a.iter_mut().find(|b| b.ts_ns == at).unwrap();
    day8.knowledge_ns = at + DAY;
    day8.close = dec!(999);
    let as_known = evaluate_over(&p, &membership(), &late, at, &mut NoModels).unwrap();
    assert_ne!(
        as_known.values["A"]["mom"], honest.values["A"]["mom"],
        "the late bar is invisible at day 8"
    );
    assert!(
        as_known.values["A"]["mom"].as_num().unwrap() < 100.0,
        "and its revised close (999) never reached the score"
    );
}

#[test]
fn a_member_with_no_bars_is_excluded_for_having_no_data() {
    let p = plan();
    let mut b = bars();
    b.remove("B");
    let run = evaluate_over(&p, &membership(), &b, t0_ns() + 10 * DAY, &mut NoModels).unwrap();
    let ex = run
        .selection
        .excluded
        .iter()
        .find(|x| x.instrument == "B")
        .unwrap();
    assert!(
        matches!(ex.why, ExclusionReason::Missing { .. }),
        "{:?}",
        ex.why
    );
    assert!(run.selection.accounts_for_universe());
}

#[test]
fn the_instant_is_the_last_close_at_or_before_the_one_asked_for() {
    let day = 86_400;
    assert_eq!(
        evaluation_instant(t0_ns() + 5 * DAY + 7, day),
        t0_ns() + 5 * DAY
    );
    assert_eq!(
        evaluation_instant(t0_ns() + 5 * DAY, day),
        t0_ns() + 5 * DAY
    );
}

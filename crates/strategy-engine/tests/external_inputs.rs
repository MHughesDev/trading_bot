//! Set P P-3.6 in the engine: a strategy reads a macro series as it stood at
//! each decision. The environment hands over vintaged rows; the engine resolves
//! them point-in-time itself, so a later vintage never reaches an earlier bar
//! and a revision to missing is missing — never the older number, never zero.

mod common;

use std::sync::Arc;

use chrono::{NaiveDate, TimeZone, Utc};
use common::*;
use dataplane::vintage::MacroObservation;
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::Value;
use rust_decimal_macros::dec;
use strategy_engine::registry::Registries;
use strategy_engine::{compile, Bar, Engine, Event, ExternalData, StdRegistries, Step};

const HOUR_NS: i64 = 3_600 * 1_000_000_000;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 6, d).unwrap()
}

/// DGS10 for 9 June: 4.0 published on the 10th, revised to missing on the 12th.
fn dgs10() -> Vec<MacroObservation> {
    vec![
        MacroObservation::derived("DGS10", day(9), day(10), Some(dec!(4.0))),
        MacroObservation::derived("DGS10", day(9), day(12), None),
    ]
}

fn plan() -> Arc<strategy_engine::plan::Plan> {
    let g = trading(
        serde_json::json!([
          {"id":"rate","op":"feature","args":{"name":{"str":"macro_dgs10"}}},
          {"id":"high","op":"gt","args":{"a":"rate","b":3.5}}
        ]),
        serde_json::json!([{"when":"high","then":{"kind":"hold"}}]),
    );
    Arc::new(
        compile(&g, &StdRegistries::new(), &ParamValues::new())
            .unwrap_or_else(|d| panic!("{d:#?}")),
    )
}

fn t0() -> i64 {
    Utc.with_ymd_and_hms(2025, 6, 8, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap()
}

fn hourly(i: usize) -> Bar {
    Bar {
        close_ns: t0() + (i as i64 + 1) * HOUR_NS,
        open: 100.0,
        high: 101.0,
        low: 99.0,
        close: 100.0,
        volume: 1.0,
    }
}

/// `rate` at each decision, `None` where missing.
fn run(engine: &mut Engine, from: usize, to: usize) -> Vec<(i64, Option<f64>)> {
    (from..to)
        .filter_map(|i| {
            let step = engine
                .on_event(
                    Event::Bar {
                        input: 0,
                        bar: hourly(i),
                        warmup: false,
                    },
                    &flat(),
                    &spec(),
                )
                .unwrap();
            match step {
                Step::Decided(t) => Some((
                    t.at_ns,
                    match &t.values["rate"] {
                        Value::Num(n) => Some(n.get()),
                        _ => None,
                    },
                )),
                _ => None,
            }
        })
        .collect()
}

/// What the resolver may know at `at_ns`: the newest vintage knowable then.
fn expected(at_ns: i64) -> Option<f64> {
    let at = chrono::DateTime::from_timestamp_nanos(at_ns);
    dgs10()
        .into_iter()
        .filter(|r| r.knowledge.time() <= at)
        .max_by_key(|r| r.vintage_date)
        .and_then(|r| r.value)
        .map(|v| v.to_string().parse().unwrap())
}

#[test]
fn a_macro_series_is_read_as_it_stood_at_each_decision() {
    let p = plan();
    assert_eq!(
        p.external_indicators(),
        vec![("macro_dgs10".to_string(), 3_600)],
        "the plan says what to load"
    );
    let mut engine = Engine::new(Arc::clone(&p)).unwrap();
    let step = engine
        .on_event(
            Event::External(ExternalData {
                macro_rows: dgs10(),
                facts: vec![],
            }),
            &flat(),
            &spec(),
        )
        .unwrap();
    assert_eq!(step, Step::Idle, "rows never decide on their own");
    let seen = run(&mut engine, 0, 24 * 7);

    let mut some = 0;
    let mut after_revision_missing = 0;
    for (at, v) in &seen {
        let want = expected(*at);
        // Within the warm-up the column is missing whatever the rows say.
        if v.is_some() || want.is_none() {
            assert_eq!(
                *v,
                want,
                "at {}",
                chrono::DateTime::from_timestamp_nanos(*at)
            );
        }
        if v.is_some() {
            some += 1;
            assert_eq!(*v, Some(4.0), "the only number this series ever had");
        }
        let revised = dgs10()[1].knowledge.time().timestamp_nanos_opt().unwrap();
        if *at >= revised {
            assert_eq!(*v, None, "revised to missing is missing, never 4.0 or 0");
            after_revision_missing += 1;
        }
    }
    assert!(some > 0, "the published value was read: {seen:?}");
    assert!(after_revision_missing > 0);
    let first = seen.iter().find(|(_, v)| v.is_some()).unwrap().0;
    assert!(
        first >= dgs10()[0].knowledge.time().timestamp_nanos_opt().unwrap(),
        "never before the vintage was knowable"
    );
}

#[test]
fn without_rows_the_series_is_missing_not_zero() {
    let mut engine = Engine::new(plan()).unwrap();
    assert!(run(&mut engine, 0, 24 * 7).iter().all(|(_, v)| v.is_none()));
}

#[test]
fn a_restored_engine_rebuilds_the_column_from_the_same_rows() {
    let p = plan();
    let rows = ExternalData {
        macro_rows: dgs10(),
        facts: vec![],
    };
    let mut straight = Engine::new(Arc::clone(&p)).unwrap();
    straight
        .on_event(Event::External(rows.clone()), &flat(), &spec())
        .unwrap();
    let k = 50;
    run(&mut straight, 0, k);
    let cp = straight.checkpoint().unwrap();
    let history: Vec<(usize, Bar)> = (0..k).map(|i| (0, hourly(i))).collect();
    let mut restored =
        Engine::restore_with_external(Arc::clone(&p), &cp, history, &spec(), Some(rows)).unwrap();
    assert_eq!(run(&mut straight, k, 24 * 7), run(&mut restored, k, 24 * 7));
}

#[test]
fn the_lookback_counts_the_publication_lag_at_the_inputs_bar_size() {
    let reg = StdRegistries::new();
    assert_eq!(reg.feature_lookback_at("macro_dgs10", 3_600), Some(25));
    assert_eq!(reg.feature_lookback_at("macro_dgs10", 86_400), Some(2));
    assert_eq!(
        reg.feature_lookback_at("rsi_14", 3_600),
        reg.feature_lookback("rsi_14")
    );
    // A name that is neither: unknown, as before.
    assert_eq!(reg.feature_lookback_at("macro_", 3_600), None);
}

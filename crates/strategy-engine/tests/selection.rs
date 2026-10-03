//! Set U U-6.2: cross-sectional evaluation of selection strategies.
//!
//! Acceptance: `cs_rank` and `cs_zscore` match a reference computation; a member
//! missing a score is excluded with its reason, never ranked as zero; results
//! are identical regardless of universe iteration order. Plus: temporal state is
//! kept per instrument, models are asked once per member, and the compiler
//! refuses what a selection cannot mean.

mod common;

use std::sync::Arc;

use common::*;
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::selection::ExclusionReason;
use domain::strategy_graph::{MissingReason, StrategyGraph, Value, ValueType};
use strategy_engine::selection::{SelectionEngine, SelectionRun, SelectionStep};
use strategy_engine::{compile, ModelOutcome, ModelResponse, StdRegistries};

#[allow(clippy::needless_pass_by_value)]
fn selection(nodes: serde_json::Value, select: serde_json::Value) -> StrategyGraph {
    graph(serde_json::json!({
        "format": "tbot.strategy/3", "kind": "selection", "name": "s",
        "asset_class": "crypto_spot_cex", "timeframe": "1h",
        "universe": {"kind": "list", "instruments": ["A", "B", "C", "D", "E"]},
        "nodes": nodes, "select": select
    }))
}

fn engine_for(g: &StrategyGraph, reg: &StdRegistries) -> SelectionEngine {
    let plan = compile(g, reg, &ParamValues::new()).unwrap_or_else(|d| panic!("{d:#?}"));
    SelectionEngine::new(Arc::new(plan)).expect("a selection engine")
}

/// Each member's own series: a shared shape, scaled and shifted per member so
/// their momenta differ. `C` trades near 40 so a price filter can exclude it;
/// `E` has three bars, too few for `roc_5`.
fn series(member: &str) -> Vec<f64> {
    let (scale, shift, n) = match member {
        "A" => (1.0, 0.0, 40),
        "B" => (1.6, -40.0, 40),
        "C" => (0.4, 0.0, 40),
        "D" => (-0.7, 160.0, 40),
        _ => (1.0, 0.0, 3),
    };
    closes(n).into_iter().map(|c| c * scale + shift).collect()
}

const MEMBERS: [&str; 5] = ["A", "B", "C", "D", "E"];

fn feed(e: &mut SelectionEngine, order: &[&str]) {
    for m in order {
        let xs = series(m);
        let skip = 40 - xs.len();
        for (i, c) in xs.into_iter().enumerate() {
            e.on_bar(m, 0, bar(i + skip, c)).unwrap();
        }
    }
}

const AT: i64 = 40 * HOUR_NS;

fn universe(order: &[&str]) -> Vec<String> {
    order.iter().map(|s| (*s).to_string()).collect()
}

fn run(e: &mut SelectionEngine, order: &[&str]) -> SelectionRun {
    match e.evaluate(AT, &universe(order), false).unwrap() {
        SelectionStep::Selected(r) => *r,
        other => panic!("{other:?}"),
    }
}

fn momentum() -> StrategyGraph {
    selection(
        serde_json::json!([
            {"id":"mom", "op":"feature",  "args":{"name":{"str":"roc_5"}}},
            {"id":"z",   "op":"cs_zscore","args":{"x":"mom"}},
            {"id":"r",   "op":"cs_rank",  "args":{"x":"mom"}},
            {"id":"ok",  "op":"gt",       "args":{"a":"close","b":50}}
        ]),
        serde_json::json!({"score":"z", "where":"ok", "top_n":2, "order":"desc",
                           "attributes":{"rank":"r"}}),
    )
}

fn num(r: &SelectionRun, m: &str, node: &str) -> Option<f64> {
    r.values[m][node].as_num()
}

#[test]
fn cs_rank_and_cs_zscore_match_a_reference_computed_from_the_members_own_values() {
    let mut e = engine_for(&momentum(), &StdRegistries::new());
    feed(&mut e, &MEMBERS);
    let r = run(&mut e, &MEMBERS);

    // The reference: every member that has a momentum, by hand.
    let present: Vec<(&str, f64)> = MEMBERS
        .iter()
        .filter_map(|m| num(&r, m, "mom").map(|x| (*m, x)))
        .collect();
    assert_eq!(present.len(), 4, "E is still warming up");
    assert_eq!(
        num(&r, "A", "mom"),
        num(&r, "C", "mom"),
        "the tie this test relies on"
    );
    let n = present.len() as f64;
    let mean = present.iter().map(|(_, x)| x).sum::<f64>() / n;
    let sd = (present.iter().map(|(_, x)| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    for (m, x) in &present {
        let z = num(&r, m, "z").unwrap();
        assert!((z - (x - mean) / sd).abs() < 1e-12, "{m}: {z}");
        // Ties share the mid-rank. `C` is `A` scaled, and `roc` does not see
        // scale, so the two tie here — on purpose.
        let above = present.iter().filter(|(_, y)| y > x).count() as f64;
        #[allow(clippy::float_cmp)]
        let equal = present.iter().filter(|(_, y)| y == x).count() as f64;
        assert_eq!(
            num(&r, m, "r"),
            Some(1.0 + above + (equal - 1.0) / 2.0),
            "{m}"
        );
    }

    // The sink: filtered, missing, ranked, cut.
    let s = &r.selection;
    assert!(s.accounts_for_universe(), "{s:#?}");
    let why = |m: &str| {
        s.excluded
            .iter()
            .find(|x| x.instrument == m)
            .map(|x| x.why.clone())
    };
    assert_eq!(
        why("C"),
        Some(ExclusionReason::Filtered),
        "C trades below 50"
    );
    assert!(
        matches!(
            why("E"),
            Some(ExclusionReason::Missing {
                reason: MissingReason::WarmingUp { .. }
            })
        ),
        "a member without a score is excluded with its reason, never ranked as zero: {:?}",
        why("E")
    );
    let mut qualified: Vec<(&str, f64)> = ["A", "B", "D"]
        .iter()
        .map(|m| (*m, num(&r, m, "z").unwrap()))
        .collect();
    qualified.sort_by(|a, b| b.1.total_cmp(&a.1));
    assert_eq!(s.instruments(), vec![qualified[0].0, qualified[1].0]);
    assert_eq!(s.selected[0].rank, 1);
    assert_eq!(
        s.selected[0].attributes["rank"].as_num(),
        num(&r, qualified[0].0, "r"),
        "attributes carry the member's own values"
    );
    assert!(matches!(
        why(qualified[2].0),
        Some(ExclusionReason::BelowTopN { rank: 3, .. })
    ));
}

#[test]
fn the_universe_order_and_the_bar_order_change_nothing() {
    let g = momentum();
    let mut a = engine_for(&g, &StdRegistries::new());
    let mut b = engine_for(&g, &StdRegistries::new());
    feed(&mut a, &MEMBERS);
    feed(&mut b, &["E", "D", "B", "C", "A"]);
    let ra = run(&mut a, &MEMBERS);
    let rb = run(&mut b, &["D", "A", "E", "C", "B", "A"]);
    assert_eq!(
        ra.digest(),
        rb.digest(),
        "byte-identical, whatever the order"
    );
    assert_eq!(ra, rb);
}

#[test]
fn temporal_state_is_kept_per_instrument() {
    let g = selection(
        serde_json::json!([
            {"id":"d", "op":"change", "args":{"x":"close","n":1}}
        ]),
        serde_json::json!({"score":"d"}),
    );
    let mut e = engine_for(&g, &StdRegistries::new());
    // Two evaluations, one bar apart: each member's change is its own.
    for m in ["A", "B"] {
        e.on_bar(m, 0, bar(0, series(m)[0])).unwrap();
    }
    let first = e.evaluate(HOUR_NS, &universe(&["A", "B"]), false).unwrap();
    let SelectionStep::Selected(first) = first else {
        panic!()
    };
    assert!(first.selection.selected.is_empty(), "no previous tick yet");
    for m in ["A", "B"] {
        e.on_bar(m, 0, bar(1, series(m)[1])).unwrap();
    }
    let SelectionStep::Selected(second) = e
        .evaluate(2 * HOUR_NS, &universe(&["A", "B"]), false)
        .unwrap()
    else {
        panic!()
    };
    for m in ["A", "B"] {
        let s = series(m);
        let d = second.values[m]["d"].as_num().unwrap();
        assert!((d - (s[1] - s[0])).abs() < 1e-9, "{m}: {d}");
    }
}

#[test]
fn a_warmup_evaluation_selects_nothing_and_calls_no_model() {
    let mut e = engine_for(&momentum(), &StdRegistries::new());
    feed(&mut e, &MEMBERS);
    assert_eq!(
        e.evaluate(AT, &universe(&MEMBERS), true).unwrap(),
        SelectionStep::Idle
    );
    assert!(matches!(
        e.evaluate(AT + HOUR_NS, &universe(&MEMBERS), false)
            .unwrap(),
        SelectionStep::Selected(_)
    ));
}

#[test]
fn a_model_is_asked_once_per_member_and_its_answer_ranks_them() {
    let c = contract(
        vec![row_input("x", &["roc_5"])],
        vec![("value", ValueType::Num)],
    );
    let reg = registries(vec![model("scorer@1", c)]);
    let g = selection(
        serde_json::json!([
            {"id":"m", "op":"model", "args":{"ref":{"str":"scorer@1"}}},
            {"id":"top", "op":"cs_rank", "args":{"x":"m.value"}}
        ]),
        serde_json::json!({"score":"m.value", "order":"asc", "attributes":{"rank":"top"}}),
    );
    let mut e = engine_for(&g, &reg);
    feed(&mut e, &MEMBERS);
    let SelectionStep::NeedModels(reqs) = e.evaluate(AT, &universe(&MEMBERS), false).unwrap()
    else {
        panic!("models first")
    };
    let mut nodes: Vec<&str> = reqs.iter().map(|r| r.node.as_str()).collect();
    nodes.sort_unstable();
    assert_eq!(
        nodes,
        ["A::m", "B::m", "C::m", "D::m"],
        "one call per member with every input present; E's row is warming up, so E is not called"
    );
    let responses = reqs
        .iter()
        .map(|r| {
            let strategy_engine::ModelInput::Row { values } = &r.inputs["x"] else {
                panic!()
            };
            ModelResponse {
                node: r.node.clone(),
                outcome: answered(vec![("value", Value::num(values["roc_5"] * 10.0, "t"))]),
            }
        })
        .collect();
    let SelectionStep::Selected(run) = e.resume(responses).unwrap() else {
        panic!()
    };
    let s = &run.selection;
    assert_eq!(s.selected.len(), 4);
    for w in s.selected.windows(2) {
        assert!(w[0].score <= w[1].score, "ascending: {s:#?}");
    }
    // The cross-sectional node downstream of the model saw every answer.
    assert_eq!(s.selected[3].attributes["rank"].as_num(), Some(1.0));
    assert!(s.excluded.iter().any(|x| x.instrument == "E"
        && matches!(
            x.why,
            ExclusionReason::Missing {
                reason: MissingReason::WarmingUp { .. }
            }
        )));
}

#[test]
fn an_unanswered_member_is_excluded_as_a_timeout() {
    let c = contract(
        vec![row_input("x", &["roc_5"])],
        vec![("value", ValueType::Num)],
    );
    let reg = registries(vec![model("scorer@1", c)]);
    let g = selection(
        serde_json::json!([{"id":"m", "op":"model", "args":{"ref":{"str":"scorer@1"}}}]),
        serde_json::json!({"score":"m.value"}),
    );
    let mut e = engine_for(&g, &reg);
    feed(&mut e, &MEMBERS);
    let SelectionStep::NeedModels(reqs) = e.evaluate(AT, &universe(&MEMBERS), false).unwrap()
    else {
        panic!()
    };
    // Answer everyone but B.
    let responses = reqs
        .iter()
        .filter(|r| r.node != "B::m")
        .map(|r| ModelResponse {
            node: r.node.clone(),
            outcome: answered(vec![("value", Value::num(1.0, "t"))]),
        })
        .collect();
    let SelectionStep::Selected(run) = e.resume(responses).unwrap() else {
        panic!()
    };
    let b = run
        .selection
        .excluded
        .iter()
        .find(|x| x.instrument == "B")
        .unwrap();
    assert!(matches!(
        b.why,
        ExclusionReason::Missing {
            reason: MissingReason::ModelTimeout { .. }
        }
    ));
    let _ = ModelOutcome::TimedOut { timeout_ms: 1 };
}

#[test]
fn a_selection_cannot_read_an_account_or_a_fixed_instrument() {
    let state = selection(
        serde_json::json!([{"id":"f", "op":"and", "args":{"a":"state.flat","b":true}}]),
        serde_json::json!({"score":"close", "where":"f"}),
    );
    let d = compile(&state, &StdRegistries::new(), &ParamValues::new()).unwrap_err();
    assert!(d.iter().any(|d| d.code == "state_in_selection"), "{d:#?}");

    let mut fixed = serde_json::to_value(momentum()).unwrap();
    fixed["inputs"] = serde_json::json!([
        {"id":"btc", "instrument":{"kind":"fixed","instrument":"BTC-USD"}, "timeframe":"1d"}
    ]);
    let fixed: StrategyGraph = serde_json::from_value(fixed).unwrap();
    let d = compile(&fixed, &StdRegistries::new(), &ParamValues::new()).unwrap_err();
    assert!(
        d.iter().any(|d| d.code == "fixed_input_in_selection"),
        "{d:#?}"
    );
}

#[test]
fn a_selection_engine_refuses_a_trading_plan() {
    let t = trading(
        serde_json::json!([{"id":"go","op":"gt","args":{"a":"close","b":1}}]),
        serde_json::json!([{"when":"go","then":{"kind":"close"}}]),
    );
    let plan = compile(&t, &StdRegistries::new(), &ParamValues::new()).unwrap();
    assert!(matches!(
        SelectionEngine::new(Arc::new(plan)),
        Err(strategy_engine::EngineError::TradingPlan)
    ));
}

//! Set U Phase 1, acceptance by acceptance (`docs/plans/plan-sets/set-U/phase-1-engine-core.md`).
//!
//! Each test names the task whose acceptance it proves. They drive the real
//! compiler and the real engine with synthetic bars and synthetic model answers —
//! no environment, no clock, no venue — which is itself the point of U-1.1: an
//! engine that needs an environment to be tested is not pure.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::{MissingReason, Num, Value, ValueType};
use rust_decimal_macros::dec;
use strategy_engine::decide::RuleOutcome;
use strategy_engine::planner::{OrderAction, PlannedStyle, RSize, ResolvedIntent};
use strategy_engine::{compile, Engine, Event, ModelOutcome, StdRegistries, Step};

fn n(x: f64) -> Value {
    Value::Num(Num::new(x).unwrap())
}

fn decided(s: &Step) -> &strategy_engine::DecisionTrace {
    match s {
        Step::Decided(t) => t,
        other => panic!("expected a decision, got {other:?}"),
    }
}

// ── U-1.1 the compiler ───────────────────────────────────────────────────────

#[test]
fn u_1_1_the_spec_examples_compile() {
    let reg = registries(vec![model(
        "vol_forecaster@12",
        contract(vec![], vec![("sigma", ValueType::Num)]),
    )]);
    let g = graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "trend_with_vol_gate",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "params": {"fast": {"type":"int","default":12,"min":5,"max":50},
                 "slow": {"type":"int","default":48,"min":20,"max":200}},
      "constraints": ["param('fast') < param('slow')"],
      "nodes": [
        {"id":"f",    "op":"ema",           "args":{"x":"close", "n":"$fast"}},
        {"id":"s",    "op":"ema",           "args":{"x":"close", "n":"$slow"}},
        {"id":"vol",  "op":"model",         "args":{"ref":{"str":"vol_forecaster@12"}}},
        {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
        {"id":"calm", "op":"lt",            "args":{"a":"vol.sigma", "b":0.03}},
        {"id":"go",   "op":"and",           "args":{"a":"up", "b":"calm"}},
        {"id":"entry","op":"and",           "args":{"a":"go", "b":"state.flat"}},
        {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
        {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
      ],
      "decide": {"rules": [
        {"when":"entry", "then":{"kind":"bracket",
           "entry":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}},
           "stop":{"amount":2,"unit":{"unit":"atr","n":14}},
           "take_profit":{"amount":4,"unit":{"unit":"atr","n":14}}}},
        {"when":"leave", "then":{"kind":"close"}}
      ]}
    }));
    let plan = compile(&g, &reg, &ParamValues::new()).unwrap_or_else(|d| panic!("{d:#?}"));
    // `ema(close, n)` was lowered to the DATA-006 indicator, not reimplemented.
    assert!(plan.inputs[0].columns.contains(&"ema_12".to_string()));
    assert!(plan.inputs[0].columns.contains(&"ema_48".to_string()));
    assert!(
        plan.inputs[0].columns.contains(&"atr_14".to_string()),
        "ATR registered for the bracket's distances"
    );
    assert_eq!(plan.rules.len(), 2);

    // The selection example compiles to a plan too (it is evaluated in Phase 6).
    let s = graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "momentum_top10",
      "asset_class": "crypto_spot_cex", "timeframe": "1d",
      "universe": {"kind":"asset_class","asset_class":"crypto_spot_cex"},
      "nodes": [
        {"id":"mom",   "op":"feature",  "args":{"name":{"str":"roc_20"}}},
        {"id":"mom_z", "op":"cs_zscore","args":{"x":"mom"}},
        {"id":"liq",   "op":"feature",  "args":{"name":{"str":"rel_volume_20"}}},
        {"id":"liq_ok","op":"gt",       "args":{"a":"liq","b":0.5}}
      ],
      "select": {"score":"mom_z", "where":"liq_ok", "top_n":10, "attributes":{"weight":"mom_z"}}
    }));
    let plan = compile(&s, &StdRegistries::new(), &ParamValues::new())
        .unwrap_or_else(|d| panic!("{d:#?}"));
    assert!(plan.select.is_some());
}

#[test]
fn u_1_1_a_cycle_is_refused_naming_every_node_in_it() {
    let g = trading(
        serde_json::json!([
          {"id":"a","op":"add","args":{"a":"c","b":1}},
          {"id":"b","op":"add","args":{"a":"a","b":1}},
          {"id":"c","op":"add","args":{"a":"b","b":1}},
          {"id":"ok","op":"gt","args":{"a":"close","b":0}}
        ]),
        serde_json::json!([{"when":"ok","then":{"kind":"hold"}}]),
    );
    let d = compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap_err();
    let cycle = d
        .iter()
        .find(|d| d.code == "cycle")
        .expect("a cycle diagnostic");
    for node in ["a", "b", "c"] {
        assert!(cycle.message.contains(node), "{cycle}");
    }
    assert!(
        cycle.fix.contains("prev"),
        "the fix says how to read last tick's value"
    );
}

#[test]
fn u_1_1_a_num_wired_into_a_bool_port_names_both_nodes_and_the_type() {
    let g = trading(
        serde_json::json!([
          {"id":"r","op":"feature","args":{"name":{"str":"rsi_14"}}},
          {"id":"both","op":"and","args":{"a":"r","b":true}}
        ]),
        serde_json::json!([{"when":"both","then":{"kind":"hold"}}]),
    );
    let d = compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap_err();
    let m = d.iter().find(|d| d.code == "type_mismatch").unwrap();
    assert_eq!(m.at, "both");
    assert!(
        m.message.contains("condition") && m.message.contains("num"),
        "{m}"
    );
    assert!(
        m.fix.contains("gt("),
        "the fix shows how to make a condition: {m}"
    );
}

#[test]
fn u_1_1_a_wrongly_typed_model_input_is_refused_against_its_contract() {
    let reg = registries(vec![model(
        "regime@1",
        contract(
            vec![scalar_input("vol", ValueType::Num)],
            vec![("label", ValueType::Cat { labels: vec![] })],
        ),
    )]);
    let g = trading(
        serde_json::json!([
          {"id":"up","op":"gt","args":{"a":"close","b":0}},
          {"id":"m","op":"model","args":{"ref":{"str":"regime@1"},"vol":"up"}},
          {"id":"go","op":"is","args":{"x":"m.label","label":{"str":"calm"}}}
        ]),
        serde_json::json!([{"when":"go","then":{"kind":"hold"}}]),
    );
    let d = compile(&g, &reg, &ParamValues::new()).unwrap_err();
    let m = d
        .iter()
        .find(|d| d.code == "model_input_type")
        .unwrap_or_else(|| panic!("{d:#?}"));
    assert!(
        m.message.contains("`vol`")
            && m.message.contains("takes num")
            && m.message.contains("bool"),
        "{m}"
    );
}

#[test]
fn u_1_1_warmup_sums_through_a_chain() {
    let g = trading(
        serde_json::json!([
          {"id":"e","op":"ema","args":{"x":"close","n":48}},
          {"id":"m","op":"rolling_mean","args":{"x":"e","n":20}},
          {"id":"go","op":"gt","args":{"a":"m","b":0}}
        ]),
        serde_json::json!([{"when":"go","then":{"kind":"hold"}}]),
    );
    let plan = compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap();
    let ema_lb = features::runtime::lookback_bars("ema_48").unwrap();
    let (_, m) = plan.node("m").unwrap();
    assert_eq!(
        m.warmup,
        ema_lb + 20,
        "the indicator's lookback plus the window over it"
    );
    assert_eq!(plan.warmup_ticks, ema_lb + 20);
}

#[test]
fn u_1_1_every_problem_is_reported_in_one_pass() {
    let g = trading(
        serde_json::json!([
          {"id":"a","op":"crosed_above","args":{"a":"close","b":"open"}},
          {"id":"b","op":"feature","args":{"name":{"str":"rsi14"}}},
          {"id":"c","op":"gt","args":{"a":"$nope","b":0}}
        ]),
        serde_json::json!([{"when":"c","then":{"kind":"hold"}}]),
    );
    let d = compile(&g, &StdRegistries::new(), &ParamValues::new()).unwrap_err();
    let codes: Vec<&str> = d.iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&"unknown_op")
            && codes.contains(&"unknown_feature")
            && codes.contains(&"unknown_param"),
        "{codes:?}"
    );
    let op = d.iter().find(|d| d.code == "unknown_op").unwrap();
    assert!(op.fix.contains("crossed_above"), "did-you-mean: {op}");
    let f = d.iter().find(|d| d.code == "unknown_feature").unwrap();
    assert!(f.fix.contains("rsi_14"), "did-you-mean: {f}");
}

// ── U-1.2 evaluation with absence ────────────────────────────────────────────

/// A sweep over every stateless numeric/boolean operator with each input missing
/// in turn: the output is never a present `num` unless the operator is an
/// absence handler or Kleene logic decides without it.
#[test]
fn u_1_2_a_missing_input_never_becomes_a_present_number() {
    use strategy_engine::eval::{eval, ArgVal};
    let missing = || ArgVal::One(Value::Missing(MissingReason::NoData { what: "m".into() }));
    let present = |port: &str| match port {
        "label" => ArgVal::One(Value::Text("up".into())),
        "cond" => ArgVal::One(Value::Bool(true)),
        _ => ArgVal::One(n(1.5)),
    };
    let handlers = [
        "coalesce",
        "is_missing",
        "missing_reason",
        "and",
        "or",
        "not",
        "if",
        "switch",
        "vote",
    ];
    for spec in domain::strategy_graph::ops::ops() {
        if spec.stateful
            || handlers.contains(&spec.name)
            || spec.inputs.is_empty()
            || spec.open_inputs
        {
            continue;
        }
        let ports: Vec<&str> = spec.inputs.iter().map(|p| p.name).collect();
        for hole in spec.inputs.iter().filter(|p| !p.literal) {
            let args: BTreeMap<String, ArgVal> = spec
                .inputs
                .iter()
                .map(|p| {
                    (
                        p.name.to_string(),
                        if p.name == hole.name {
                            missing()
                        } else {
                            present(p.name)
                        },
                    )
                })
                .collect();
            let out = eval(spec.name, &ports, &args);
            assert!(
                !matches!(out, Value::Num(_)),
                "{} with {} missing gave {out:?}",
                spec.name,
                hole.name
            );
        }
    }
}

#[test]
fn u_1_2_if_and_coalesce_behave_with_absence() {
    use strategy_engine::eval::{eval, ArgVal};
    let m = ArgVal::One(Value::Missing(MissingReason::NoData { what: "m".into() }));
    let args = |xs: Vec<(&str, ArgVal)>| {
        xs.into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<BTreeMap<_, _>>()
    };
    assert!(eval(
        "if",
        &["cond", "then", "else"],
        &args(vec![
            ("cond", m.clone()),
            ("then", ArgVal::One(n(1.0))),
            ("else", ArgVal::One(n(2.0)))
        ])
    )
    .is_missing());
    assert_eq!(
        eval(
            "coalesce",
            &["x", "default"],
            &args(vec![("x", m), ("default", ArgVal::One(n(3.0)))])
        )
        .as_num(),
        Some(3.0)
    );
}

// ── U-1.3 indicators inside the engine ───────────────────────────────────────

#[test]
fn u_1_3_indicators_match_the_feature_engine_bar_for_bar_from_bars_alone() {
    let g = trading(
        serde_json::json!([
          {"id":"fast","op":"ema","args":{"x":"close","n":7}},
          {"id":"slow","op":"ema","args":{"x":"close","n":21}},
          {"id":"up","op":"gt","args":{"a":"fast","b":"slow"}}
        ]),
        serde_json::json!([{"when":"up","then":{"kind":"hold"}}]),
    );
    let mut e = engine(&g, &StdRegistries::new());
    let cs = closes(250);
    let steps = drive(&mut e, &cs, &|_| ModelOutcome::Abstained {
        detail: "none".into(),
    });

    // Reference: the DATA-006 routine over the same bars.
    let rows: Vec<dataplane::feature::FeatureRow> = cs
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let b = bar(i, *c);
            dataplane::feature::FeatureRow {
                ts_ns: b.close_ns,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
            }
        })
        .collect();
    let f7 = features::runtime::feature("ema_7").unwrap();
    let f21 = features::runtime::feature("ema_21").unwrap();
    let lb21 = features::runtime::lookback_bars("ema_21").unwrap() as usize;
    for (i, s) in steps.iter().enumerate() {
        let t = decided(s);
        let want7 = features::runtime::value_at(f7.as_ref(), &rows[..=i], i);
        let want21 = features::runtime::value_at(f21.as_ref(), &rows[..=i], i);
        assert_eq!(t.values["fast"].as_num(), want7, "bar {i}: ema_7");
        assert_eq!(t.values["slow"].as_num(), want21, "bar {i}: ema_21");
        if i + 1 < lb21 {
            // Before warm-up the comparison is missing(warming_up) — not false.
            assert!(
                matches!(
                    t.values["up"],
                    Value::Missing(MissingReason::WarmingUp { .. })
                ),
                "bar {i}: {:?}",
                t.values["up"]
            );
            assert!(matches!(t.rules[&0], RuleOutcome::Unknown { .. }));
        } else {
            assert!(matches!(t.values["up"], Value::Bool(_)), "bar {i}");
        }
    }
}

/// The structural half of U-1.3: nothing outside the engine can supply an
/// indicator value. `history` is a private module and no public item takes one.
#[test]
fn u_1_3_there_is_no_public_way_to_write_an_indicator_value() {
    let lib = include_str!("../src/lib.rs");
    assert!(
        lib.contains("mod history;") && !lib.contains("pub mod history"),
        "history must stay private"
    );
    let engine = include_str!("../src/engine.rs");
    for forbidden in [
        "pub fn set_feature",
        "pub fn push_feature",
        "pub fn write_feature",
        "Feature {",
        "FeatureValue",
    ] {
        if forbidden == "Feature {" {
            // The `Event` enum must have no feature-carrying variant.
            let event = engine
                .split("pub enum Event")
                .nth(1)
                .unwrap()
                .split("pub enum Step")
                .next()
                .unwrap();
            assert!(
                !event.contains(forbidden),
                "Event carries an indicator value"
            );
        } else {
            assert!(!engine.contains(forbidden), "{forbidden}");
        }
    }
}

// ── U-1.4 temporal operators advance once per tick ───────────────────────────

#[test]
fn u_1_4_a_model_resume_does_not_advance_temporal_state() {
    let reg = registries(vec![
        model("a@1", contract(vec![], vec![("x", ValueType::Num)])),
        model(
            "b@1",
            contract(
                vec![scalar_input("x", ValueType::Num)],
                vec![("y", ValueType::Num)],
            ),
        ),
    ]);
    let g = trading(
        serde_json::json!([
          {"id":"pos","op":"gt","args":{"a":"close","b":0}},
          {"id":"cnt","op":"count_true","args":{"cond":"pos","n":3}},
          {"id":"a","op":"model","args":{"ref":{"str":"a@1"}}},
          {"id":"b","op":"model","args":{"ref":{"str":"b@1"},"x":"a.x"}},
          {"id":"go","op":"gt","args":{"a":"b.y","b":0}}
        ]),
        serde_json::json!([{"when":"go","then":{"kind":"hold"}}]),
    );
    let mut e = engine(&g, &reg);
    let mut step = e
        .on_event(
            Event::Bar {
                input: 0,
                bar: bar(0, 100.0),
                warmup: false,
            },
            &flat(),
            &spec(),
        )
        .unwrap();
    let mut layers = 0;
    while let Step::NeedModels(reqs) = &step {
        layers += 1;
        let r = reqs
            .iter()
            .map(|r| strategy_engine::ModelResponse {
                node: r.node.clone(),
                outcome: answered(vec![(if r.node == "a" { "x" } else { "y" }, n(1.0))]),
            })
            .collect();
        step = e.resume(r).unwrap();
    }
    assert_eq!(layers, 2, "a, then b");
    let t = decided(&step);
    // One tick has passed: the window holds one value of three, however many
    // times the tick was resumed.
    assert_eq!(
        t.values["cnt"],
        Value::Missing(MissingReason::WarmingUp { needed: 3, have: 1 })
    );
}

// ── U-1.5 state from the portfolio view ──────────────────────────────────────

#[test]
fn u_1_5_every_state_field_is_read_from_the_view() {
    let fields = [
        "flat",
        "long",
        "short",
        "position",
        "position_fraction",
        "entry_price",
        "bars_in_position",
        "unrealized_return",
        "realized_pnl_today",
        "equity",
        "cash",
        "open_orders",
        "mark",
    ];
    let nodes: Vec<serde_json::Value> = fields
        .iter()
        .map(|f| serde_json::json!({"id": format!("s_{f}"), "op":"is_missing", "args":{"x": format!("state.{f}")}}))
        .collect();
    let g = trading(
        serde_json::Value::Array(nodes),
        serde_json::json!([{"when":"s_flat","then":{"kind":"hold"}}]),
    );
    let mut e = engine(&g, &StdRegistries::new());
    let view = strategy_engine::PortfolioView {
        position: dec!(2),
        avg_entry_price: Some(dec!(90)),
        bars_in_position: 3,
        high_water: Some(dec!(105)),
        equity: dec!(1000),
        cash: dec!(800),
        realized_pnl_today: dec!(-5),
        mark: Some(dec!(100)),
        open_orders: vec![],
        last_fill_price: Some(dec!(90)),
    };
    let s = e
        .on_event(
            Event::Bar {
                input: 0,
                bar: bar(0, 100.0),
                warmup: false,
            },
            &view,
            &spec(),
        )
        .unwrap();
    let t = decided(&s);
    for f in fields {
        assert_eq!(
            t.values[&format!("s_{f}")],
            Value::Bool(false),
            "state.{f} is present when the view has it"
        );
    }
    // And the engine holds no position of its own.
    for src in [
        include_str!("../src/engine.rs"),
        include_str!("../src/plan.rs"),
    ] {
        assert!(
            !src.contains("position: Decimal") && !src.contains("my_position"),
            "the engine keeps no position"
        );
    }
}

// ── U-1.6 models by request and resume ───────────────────────────────────────

fn chain_graph() -> (domain::strategy_graph::StrategyGraph, StdRegistries) {
    let reg = registries(vec![
        model(
            "dir@1",
            contract(
                vec![row_input("features", &["rsi_14", "roc_20"])],
                vec![(
                    "probs",
                    ValueType::Probs {
                        labels: vec!["down".into(), "flat".into(), "up".into()],
                    },
                )],
            ),
        ),
        model(
            "size@1",
            contract(
                vec![scalar_input("p_up", ValueType::Num)],
                vec![("fraction", ValueType::Num)],
            ),
        ),
    ]);
    let g = trading(
        serde_json::json!([
          {"id":"dir","op":"model","args":{"ref":{"str":"dir@1"}}},
          {"id":"p_up","op":"prob","args":{"p":"dir.probs","label":{"str":"up"}}},
          {"id":"size","op":"model","args":{"ref":{"str":"size@1"},"p_up":"p_up"}},
          {"id":"r","op":"feature","args":{"name":{"str":"rsi_14"}}},
          {"id":"trend","op":"gt","args":{"a":"r","b":50}},
          {"id":"bullish","op":"gt","args":{"a":"p_up","b":0.6}},
          {"id":"frac","op":"if","args":{"cond":"trend","then":"size.fraction","else":0.1}},
          {"id":"go","op":"and","args":{"a":"bullish","b":"state.flat"}}
        ]),
        serde_json::json!([{"when":"go","then":{"kind":"target","size":{"mode":"equity_fraction","x":"frac"}}}]),
    );
    (g, reg)
}

fn probs(up: f64) -> Value {
    Value::Probs {
        labels: vec!["down".into(), "flat".into(), "up".into()],
        p: vec![
            Num::new((1.0 - up) / 2.0).unwrap(),
            Num::new((1.0 - up) / 2.0).unwrap(),
            Num::new(up).unwrap(),
        ],
    }
}

#[test]
fn u_1_6_a_chain_of_two_models_resolves_in_layers_and_drives_an_order() {
    let (g, reg) = chain_graph();
    let mut e = engine(&g, &reg);
    let cs = closes(120);
    let mut layers_seen = Vec::new();
    let steps = drive(&mut e, &cs, &|r| match r.node.as_str() {
        "dir" => answered(vec![("probs", probs(0.8))]),
        _ => answered(vec![("fraction", n(0.3))]),
    });
    // Re-run one late tick by hand to observe the layers.
    let mut e2 = engine(&g, &reg);
    for (i, c) in cs.iter().enumerate() {
        let mut s = e2
            .on_event(
                Event::Bar {
                    input: 0,
                    bar: bar(i, *c),
                    warmup: false,
                },
                &flat(),
                &spec(),
            )
            .unwrap();
        while let Step::NeedModels(reqs) = &s {
            if i == cs.len() - 1 {
                layers_seen.push(reqs.iter().map(|r| r.node.clone()).collect::<Vec<_>>());
            }
            let resp = reqs
                .iter()
                .map(|r| strategy_engine::ModelResponse {
                    node: r.node.clone(),
                    outcome: if r.node == "dir" {
                        answered(vec![("probs", probs(0.8))])
                    } else {
                        answered(vec![("fraction", n(0.3))])
                    },
                })
                .collect();
            s = e2.resume(resp).unwrap();
        }
    }
    assert_eq!(
        layers_seen,
        vec![vec!["dir".to_string()], vec!["size".to_string()]],
        "dir first, then size"
    );

    // The classifier's probabilities drove the decision without being reshaped,
    // and the second model's fraction set the size.
    let last = decided(steps.last().unwrap());
    assert_eq!(last.values["p_up"].as_num(), Some(0.8));
    let (_, intent) = &last.intents[0];
    let trend = matches!(last.values["trend"], Value::Bool(true));
    let want = if trend { 0.3 } else { 0.1 };
    assert_eq!(
        *intent,
        ResolvedIntent::Target {
            size: RSize::EquityFraction { x: want },
            via: None
        }
    );
    assert!(matches!(&last.actions[0], OrderAction::Submit(o) if o.style == PlannedStyle::Market));
}

#[test]
fn u_1_6_an_abstain_upstream_means_the_downstream_model_is_never_called() {
    let (g, reg) = chain_graph();
    let mut e = engine(&g, &reg);
    let cs = closes(120);
    let mut size_calls = 0;
    for (i, c) in cs.iter().enumerate() {
        let mut s = e
            .on_event(
                Event::Bar {
                    input: 0,
                    bar: bar(i, *c),
                    warmup: false,
                },
                &flat(),
                &spec(),
            )
            .unwrap();
        while let Step::NeedModels(reqs) = &s {
            size_calls += reqs.iter().filter(|r| r.node == "size").count();
            let resp = reqs
                .iter()
                .map(|r| strategy_engine::ModelResponse {
                    node: r.node.clone(),
                    outcome: ModelOutcome::Abstained {
                        detail: "no view".into(),
                    },
                })
                .collect();
            s = e.resume(resp).unwrap();
        }
        let t = decided(&s);
        assert!(t.values["p_up"].is_missing());
        assert!(
            t.intents.is_empty(),
            "nothing trades on a model that abstained"
        );
    }
    assert_eq!(
        size_calls, 0,
        "a model whose input is missing is not called"
    );
}

#[test]
fn u_1_6_model_inputs_are_never_zero_filled() {
    let (g, reg) = chain_graph();
    let mut e = engine(&g, &reg);
    // First bar: rsi_14 and roc_20 are still warming up, so `dir` cannot be
    // assembled and must not be requested at all.
    let s = e
        .on_event(
            Event::Bar {
                input: 0,
                bar: bar(0, 100.0),
                warmup: false,
            },
            &flat(),
            &spec(),
        )
        .unwrap();
    let t = decided(&s);
    assert!(
        t.model_requests.is_empty(),
        "no request with an incomplete row"
    );
    assert!(matches!(
        t.values["dir"],
        Value::Missing(MissingReason::WarmingUp { .. })
    ));
}

#[test]
fn u_1_6_the_same_answers_in_a_different_order_give_the_same_decisions() {
    let reg = registries(vec![
        model("a@1", contract(vec![], vec![("x", ValueType::Num)])),
        model("b@1", contract(vec![], vec![("x", ValueType::Num)])),
    ]);
    let g = trading(
        serde_json::json!([
          {"id":"a","op":"model","args":{"ref":{"str":"a@1"}}},
          {"id":"b","op":"model","args":{"ref":{"str":"b@1"}}},
          {"id":"s","op":"add","args":{"a":"a.x","b":"b.x"}},
          {"id":"go","op":"gt","args":{"a":"s","b":1}}
        ]),
        serde_json::json!([{"when":"go","then":{"kind":"target","size":{"mode":"units","x":"s"}}}]),
    );
    let run = |reverse: bool| {
        let mut e = engine(&g, &reg);
        let mut s = e
            .on_event(
                Event::Bar {
                    input: 0,
                    bar: bar(0, 100.0),
                    warmup: false,
                },
                &flat(),
                &spec(),
            )
            .unwrap();
        let Step::NeedModels(reqs) = &s else {
            panic!("both models requested in one layer")
        };
        assert_eq!(reqs.len(), 2);
        let mut resp: Vec<_> = reqs
            .iter()
            .map(|r| strategy_engine::ModelResponse {
                node: r.node.clone(),
                outcome: answered(vec![("x", n(if r.node == "a" { 0.7 } else { 0.9 }))]),
            })
            .collect();
        if reverse {
            resp.reverse();
        }
        s = e.resume(resp).unwrap();
        let t = decided(&s).clone();
        (t.intents.clone(), t.actions.clone(), t.values.clone())
    };
    assert_eq!(run(false), run(true));
}

// ── U-1.7 rules to intents ───────────────────────────────────────────────────

#[test]
fn u_1_7_a_size_that_depends_on_a_missing_model_holds_with_the_reason() {
    let reg = registries(vec![model(
        "frac@1",
        contract(vec![], vec![("f", ValueType::Num)]),
    )]);
    let g = trading(
        serde_json::json!([
          {"id":"m","op":"model","args":{"ref":{"str":"frac@1"}}},
          {"id":"always","op":"ge","args":{"a":"close","b":0}}
        ]),
        serde_json::json!([{"when":"always","then":{"kind":"target","size":{"mode":"equity_fraction","x":"m.f"}}}]),
    );
    let mut e = engine(&g, &reg);
    let steps = drive(&mut e, &[100.0], &|_| ModelOutcome::TimedOut {
        timeout_ms: 100,
    });
    let t = decided(&steps[0]);
    assert!(t.intents.is_empty() && t.actions.is_empty());
    assert_eq!(
        t.rules[&0],
        RuleOutcome::Held {
            reasons: vec![MissingReason::ModelTimeout {
                model: "frac@1".into(),
                timeout_ms: 100
            }]
        }
    );
}

#[test]
fn u_1_7_first_match_wins_within_a_group_and_groups_are_independent() {
    let g = trading(
        serde_json::json!([{"id":"t","op":"ge","args":{"a":"close","b":0}}]),
        serde_json::json!([
          {"when":"t","then":{"kind":"target","size":{"mode":"units","x":1}}},
          {"when":"t","then":{"kind":"target","size":{"mode":"units","x":2}}},
          {"when":"t","group":"housekeeping","then":{"kind":"cancel","tag":"stale"}}
        ]),
    );
    let mut e = engine(&g, &StdRegistries::new());
    let steps = drive(&mut e, &[100.0], &|_| ModelOutcome::Abstained {
        detail: String::new(),
    });
    let t = decided(&steps[0]);
    assert_eq!(t.rules[&0], RuleOutcome::Fired);
    assert_eq!(t.rules[&1], RuleOutcome::Shadowed);
    assert_eq!(
        t.rules[&2],
        RuleOutcome::Fired,
        "another group acts on the same tick"
    );
}

// ── U-1.9 determinism ────────────────────────────────────────────────────────

#[test]
fn u_1_9_the_same_inputs_give_byte_identical_traces() {
    let (g, reg) = chain_graph();
    let run = || {
        let mut e = engine(&g, &reg);
        drive(&mut e, &closes(150), &|r| {
            if r.node == "dir" {
                answered(vec![("probs", probs(0.7))])
            } else {
                answered(vec![("fraction", n(0.2))])
            }
        })
        .iter()
        .map(|s| serde_json::to_vec(decided(s)).unwrap())
        .collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert_eq!(a.len(), 150);
    assert_eq!(a, b, "byte-identical");
}

/// The structural half: no hash-ordered collection anywhere in the engine, so no
/// output can depend on iteration order — stronger than any one permutation test.
#[test]
fn u_1_9_the_engine_uses_no_hash_ordered_collection() {
    let srcs = [
        ("compile.rs", include_str!("../src/compile.rs")),
        ("decide.rs", include_str!("../src/decide.rs")),
        ("engine.rs", include_str!("../src/engine.rs")),
        ("eval.rs", include_str!("../src/eval.rs")),
        ("history.rs", include_str!("../src/history.rs")),
        ("models.rs", include_str!("../src/models.rs")),
        ("plan.rs", include_str!("../src/plan.rs")),
        ("planner.rs", include_str!("../src/planner.rs")),
        ("portfolio.rs", include_str!("../src/portfolio.rs")),
        ("registry.rs", include_str!("../src/registry.rs")),
        ("temporal.rs", include_str!("../src/temporal.rs")),
        ("trace.rs", include_str!("../src/trace.rs")),
    ];
    for (name, src) in srcs {
        for bad in ["HashMap", "HashSet"] {
            assert!(!src.contains(bad), "{name} uses {bad}");
        }
    }
}

// ── purity: the engine reaches nothing an environment owns ───────────────────

/// Set U §5: `strategy-engine` may not depend — directly or transitively — on
/// `execution`, `risk`, `storage`, `model-registry` or `tokio`. Read from
/// Cargo.lock, so it covers the transitive graph without running cargo.
#[test]
fn the_engine_depends_on_nothing_an_environment_owns() {
    let lock = include_str!("../../../Cargo.lock");
    let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in lock.split("[[package]]").skip(1) {
        let name = pkg
            .lines()
            .find_map(|l| l.strip_prefix("name = "))
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();
        let list: Vec<String> = pkg
            .split("dependencies = [")
            .nth(1)
            .map_or("", |d| d.split(']').next().unwrap_or(""))
            .lines()
            .filter_map(|l| {
                l.trim()
                    .trim_end_matches(',')
                    .trim_matches('"')
                    .split(' ')
                    .next()
                    .map(str::to_string)
            })
            .filter(|s| !s.is_empty())
            .collect();
        deps.entry(name).or_default().extend(list);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec!["strategy-engine".to_string()];
    while let Some(p) = stack.pop() {
        if !seen.insert(p.clone()) {
            continue;
        }
        stack.extend(deps.get(&p).cloned().unwrap_or_default());
    }
    // Positive control: a parser that found nothing would pass the check below
    // vacuously.
    for expected in ["domain", "features", "dataplane", "serde"] {
        assert!(
            seen.contains(expected),
            "the lockfile walk did not reach `{expected}` — the guard is blind"
        );
    }
    for forbidden in [
        "execution",
        "risk",
        "storage",
        "model-registry",
        "tokio",
        "sqlx",
        "reqwest",
    ] {
        assert!(
            !seen.contains(forbidden),
            "strategy-engine reaches `{forbidden}`"
        );
    }
}

#[test]
fn a_trading_engine_refuses_a_selection_plan() {
    let s = graph(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "selection", "name": "s", "asset_class": "x", "timeframe": "1d",
      "universe": {"kind":"list","instruments":["A","B"]},
      "nodes": [{"id":"m","op":"feature","args":{"name":{"str":"roc_20"}}}],
      "select": {"score":"m"}
    }));
    let plan = compile(&s, &StdRegistries::new(), &ParamValues::new()).unwrap();
    assert!(matches!(
        Engine::new(Arc::new(plan)),
        Err(strategy_engine::EngineError::SelectionPlan)
    ));
}

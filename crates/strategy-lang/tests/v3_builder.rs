//! Set U U-7.4 (builder half) and U-7.2 (the builder door): builder operations
//! produce the same graph as the equivalent SLv3 text, and the same mistake is
//! the same finding through text, builder and JSON.

use domain::strategy_def::params::ParamValues;
use serde_json::json;
use strategy_engine::StdRegistries;
use strategy_lang::v3::{build, parse, validate, BuildOp, Source};

fn ops(v: serde_json::Value) -> Vec<BuildOp> {
    serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"))
}

const TEXT: &str = r#"
strategy ema_cross kind trading asset_class crypto_spot_cex timeframe 1h
param fast: int(default: 12, min: 5, max: 50)
let f = ema(close, $fast)
let s = ema(close, 48)
let up = crossed_above(f, s)
let entry = up and state.flat
let label = prob(f, "up")
when entry -> target(size: equity_fraction(x: 0.5))
rule exit group out when crossed_below(f, s) -> close
"#;

fn steps() -> Vec<BuildOp> {
    ops(json!([
        {"op": "new", "name": "ema_cross", "kind": "trading", "asset_class": "crypto_spot_cex", "timeframe": "1h"},
        {"op": "set_param", "name": "fast", "spec": {"type": "int", "default": 12, "min": 5, "max": 50}},
        {"op": "add_node", "id": "f", "operator": "ema"},
        {"op": "connect", "from": "close", "to": "f", "port": "x"},
        {"op": "connect", "from": "$fast", "to": "f", "port": "n"},
        {"op": "add_node", "id": "s", "operator": "ema", "args": {"x": "close"}},
        {"op": "set_arg", "node": "s", "port": "n", "value": 48},
        {"op": "add_node", "id": "up", "operator": "crossed_above"},
        {"op": "connect", "from": "f", "to": "up", "port": "a"},
        {"op": "connect", "from": "s", "to": "up", "port": "b"},
        {"op": "add_node", "id": "entry", "operator": "and", "args": {"a": "up", "b": "state.flat"}},
        {"op": "add_node", "id": "label", "operator": "prob", "args": {"p": "f"}},
        {"op": "set_arg", "node": "label", "port": "label", "value": "up"},
        {"op": "add_rule", "when": "entry", "then": {"kind": "target", "size": {"mode": "equity_fraction", "x": 0.5}}},
        {"op": "add_rule", "name": "exit", "group": "out",
         "when": {"op": "crossed_below", "args": {"a": "f", "b": "s"}}, "then": {"kind": "close"}}
    ]))
}

#[test]
fn builder_steps_produce_the_same_graph_as_the_text() {
    let built = build(None, &steps()).unwrap_or_else(|f| panic!("{f:#?}"));
    let parsed = parse(TEXT).unwrap();
    assert_eq!(built, parsed);
    assert_eq!(built.revision().unwrap(), parsed.revision().unwrap());
}

#[test]
fn a_builder_string_is_a_literal_and_a_connection_is_a_reference() {
    let g = build(None, &steps()).unwrap();
    let label = g.node("label").unwrap();
    assert_eq!(
        serde_json::to_value(&label.args["label"]).unwrap(),
        json!({"str": "up"})
    );
    assert_eq!(serde_json::to_value(&label.args["p"]).unwrap(), json!("f"));
}

#[test]
fn editing_continues_from_an_existing_graph() {
    let g = parse(TEXT).unwrap();
    let edited = build(
        Some(&g),
        &ops(json!([
            {"op": "remove_node", "id": "label"},
            {"op": "set_arg", "node": "s", "port": "n", "value": 50}
        ])),
    )
    .unwrap();
    assert!(edited.node("label").is_none());
    assert_eq!(
        edited.node("s").unwrap().args["n"].literal_num(),
        Some(50.0)
    );
    assert_ne!(edited.revision().unwrap(), g.revision().unwrap());
}

#[test]
fn a_bad_step_is_named_by_its_index() {
    let err = build(
        None,
        &ops(json!([{"op": "add_node", "id": "x", "operator": "ema"}])),
    )
    .unwrap_err();
    assert_eq!(err.at, "ops[0]");
    assert_eq!(err.code, "builder_no_strategy");
    let mut s = steps();
    s.push(
        serde_json::from_value(json!({"op": "add_node", "id": "f", "operator": "sma"})).unwrap(),
    );
    let err = build(None, &s).unwrap_err();
    assert_eq!(err.code, "duplicate_node_id");
    assert_eq!(err.at, format!("ops[{}]", s.len() - 1));
}

#[test]
fn the_same_mistake_is_the_same_finding_through_text_builder_and_json() {
    // An unknown feature, and a stop placed zero distance away.
    let text = "strategy s kind trading asset_class crypto_spot_cex timeframe 1h\n\
                let x = feature(\"rsi14\")\n\
                when x > 1 -> protect(stop: {amount: 0, unit: pct})\n";
    let built = build(None, &ops(json!([
        {"op": "new", "name": "s", "kind": "trading", "asset_class": "crypto_spot_cex", "timeframe": "1h"},
        {"op": "add_node", "id": "x", "operator": "feature"},
        {"op": "set_arg", "node": "x", "port": "name", "value": "rsi14"},
        {"op": "add_rule", "when": {"op": "gt", "args": {"a": "x", "b": 1}},
         "then": {"kind": "protect", "stop": {"amount": 0, "unit": {"unit": "pct"}}}}
    ])))
    .unwrap();
    let json = serde_json::to_value(&built).unwrap();
    let run = |s: Source<'_>| {
        validate(s, &StdRegistries::new(), &ParamValues::new(), None)
            .err()
            .unwrap_or_default()
    };
    let from_text = run(Source::Text(text));
    let from_builder = run(Source::Graph(built));
    let from_json = run(Source::Json(&json));
    assert!(!from_text.is_empty());
    assert_eq!(from_text, from_builder);
    assert_eq!(from_text, from_json);
    let codes: Vec<&str> = from_text.iter().map(|f| f.code.as_str()).collect();
    assert!(codes.contains(&"unknown_feature"), "{codes:?}");
}

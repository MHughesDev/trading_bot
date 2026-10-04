//! Set U U-7.3: v1 → v3.
//!
//! Acceptance: every stored definition translates or is refused naming the
//! unsupported construct; each translation carries its behaviour-change list;
//! a translated EMA cross compiles (here) and backtests
//! (`crates/backtest/tests/translated_v1.rs`).

use domain::strategy_def::params::ParamValues;
use domain::strategy_def::StrategyDefinition;
use strategy_engine::StdRegistries;
use strategy_lang::v3::translate_v1::translate;
use strategy_lang::v3::{parse, validate, Source};
use strategy_lang::{skeleton, text_to_definition, HypothesisFamily};

fn def(v: serde_json::Value) -> StrategyDefinition {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}\n{v:#}"))
}

/// The v1 definitions this repository holds: the runtime's fixture, and every
/// SLv2 skeleton compiled to v1.
fn corpus() -> Vec<(String, StrategyDefinition)> {
    let mut out = vec![(
        "fixtures/default_ema.json".to_string(),
        def(serde_json::from_str(include_str!(
            "../../strategy-runtime/fixtures/default_ema.json"
        ))
        .unwrap()),
    )];
    for f in HypothesisFamily::ALL {
        if let Ok(s) = skeleton(f) {
            let d = text_to_definition(s.text).unwrap_or_else(|e| panic!("{f:?}: {e:?}"));
            out.push((format!("skeleton {f:?}"), d));
        }
    }
    out
}

#[test]
fn every_stored_definition_translates_or_is_refused_by_name() {
    let corpus = corpus();
    assert!(
        corpus.len() >= 4,
        "the corpus is the fixture and the skeletons"
    );
    for (name, d) in corpus {
        match translate(&d, "1h") {
            Ok(t) => {
                assert!(
                    t.changes.iter().any(|c| c.code == "fills_next_bar_open"),
                    "{name}"
                );
                assert_eq!(
                    parse(&t.text).unwrap(),
                    t.graph,
                    "{name}: the translation round-trips as SLv3"
                );
                assert_eq!(t.graph.name, d.strategy_id);
                assert_eq!(
                    t.graph.params, d.parameters,
                    "{name}: parameters carry over unchanged"
                );
            }
            Err(r) => {
                assert!(!r.construct.is_empty() && r.why.len() > 10, "{name}: {r}");
            }
        }
    }
}

#[test]
fn a_translated_ema_cross_compiles_with_entries_exits_and_its_change_list() {
    let d = def(serde_json::json!({
      "strategy_id": "ema_cross_v1", "definition_version": "1.4", "asset_class": "crypto_spot_cex",
      "inputs": [{"lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"]}],
      "parameters": {"fast": {"type": "int", "default": 7, "min": 3, "max": 20}},
      "nodes": [
        {"id": "up", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')"},
        {"id": "dn", "type": "condition", "expr": "feature('ema_7') < feature('ema_21') - 0.5"},
        {"id": "s1", "type": "signal", "when": "up", "emit": "long"},
        {"id": "s2", "type": "signal", "when": "dn", "emit": "exit"}
      ],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "fixed", "size": "0.5"}}],
      "risk_overrides": {"max_order_rate_per_minute": 6},
      "exits": {"mode": "rules", "rules": [
        {"rule": "stop_loss", "fraction": "0.02"},
        {"rule": "take_profit", "fraction": "1.5", "unit": {"measure": "atr", "period": 14}},
        {"rule": "max_holding_bars", "bars": 48},
        {"rule": "on_signal", "emit": "exit"}
      ]}
    }));
    let t = translate(&d, "1h").expect("translates");
    let text = &t.text;
    assert!(
        text.contains("let dn = feature(\"ema_7\") < feature(\"ema_21\") - 0.5"),
        "{text}"
    );
    assert!(
        text.contains(
            "when up and not prev(up, 1.0) -> order(side: buy, size: units(x: 0.5), style: market, tif: gtc)"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "protect(stop: {amount: 2.0, unit: pct}, take_profit: {amount: 1.5, unit: atr(n: 14)})"
        ),
        "{text}"
    );
    assert!(
        text.contains("when state.bars_in_position >= 48.0 -> close"),
        "{text}"
    );
    assert!(
        text.contains("when dn and not state.flat -> close"),
        "{text}"
    );
    assert!(text.contains("risk {max_orders_per_min: 6}"), "{text}");
    let codes: Vec<&str> = t.changes.iter().map(|c| c.code).collect();
    assert!(
        codes.contains(&"exits_are_orders") && codes.contains(&"risk_gate_applied"),
        "{codes:?}"
    );
    validate(
        Source::Graph(t.graph.clone()),
        &StdRegistries::new(),
        &ParamValues::new(),
        None,
    )
    .unwrap_or_else(|f| panic!("the translation compiles: {f:#?}"));
    assert_eq!(t.graph.params.len(), 1, "the parameter carried over");
}

#[test]
fn a_plain_ema_cross_translates_to_a_strategy_that_compiles() {
    let d = def(serde_json::json!({
      "strategy_id": "ema_plain", "definition_version": "1.0", "asset_class": "crypto_spot_cex",
      "inputs": [],
      "nodes": [
        {"id": "up", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')"},
        {"id": "s1", "type": "signal", "when": "up", "emit": "long"}
      ],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "percent_of_balance", "size": "0.1"}}]
    }));
    let t = translate(&d, "1h").unwrap();
    let v = validate(
        Source::Graph(t.graph),
        &StdRegistries::new(),
        &ParamValues::new(),
        None,
    )
    .unwrap_or_else(|f| panic!("{f:#?}"));
    assert!(v.text.contains("equity_fraction(x: 0.1)"), "{}", v.text);
}

/// Found migrating the stored strategies (U-8.5): v1 multiplies conditions as
/// 1/0 — `triple_ema` and `ema_rsi_trend` are written so — and v3 does not let
/// a truth value stand for a number. The translation writes v1's meaning out.
#[test]
fn conditions_multiplied_as_numbers_translate_to_what_v1_meant() {
    let d = def(serde_json::json!({
      "strategy_id": "triple_ema", "definition_version": "1.0", "asset_class": "crypto_spot_cex",
      "inputs": [],
      "nodes": [
        {"id": "c_bull", "type": "condition",
         "expr": "(feature('ema_7') > feature('ema_21')) * (feature('ema_21') > feature('ema_50')) > 0"},
        {"id": "s1", "type": "signal", "when": "c_bull", "emit": "long"}
      ],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "fixed", "size": "0.1"}}]
    }));
    let t = translate(&d, "1h").unwrap();
    let v = validate(
        Source::Graph(t.graph),
        &StdRegistries::new(),
        &ParamValues::new(),
        None,
    )
    .unwrap_or_else(|f| panic!("{f:#?}"));
    assert!(
        v.text.contains("if(feature(\"ema_7\") > feature(\"ema_21\"), 1"),
        "{}",
        v.text
    );
}

/// Found migrating the stored strategies (U-8.5): two stored sweeps name their
/// EMAs by parameter. That is the operator with a parameter port, and it
/// compiles to the same indicator column.
#[test]
fn a_feature_named_by_parameter_translates_to_the_operator_with_its_port() {
    let d = def(serde_json::json!({
      "strategy_id": "ema_sweep", "definition_version": "1.2", "asset_class": "crypto_spot_cex",
      "inputs": [],
      "parameters": {
        "fast": {"type": "int", "default": 7, "min": 3, "max": 20},
        "slow": {"type": "int", "default": 21, "min": 10, "max": 60}
      },
      "nodes": [
        {"id": "n1", "type": "condition", "expr": "feature('ema_{{fast}}') > feature('ema_{{slow}}')"},
        {"id": "s1", "type": "signal", "when": "n1", "emit": "long"}
      ],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "fixed", "size": "0.1"}}]
    }));
    let t = translate(&d, "1h").unwrap_or_else(|r| panic!("{r}"));
    assert!(t.text.contains("ema(close, $fast) > ema(close, $slow)"), "{}", t.text);
    validate(
        Source::Graph(t.graph),
        &StdRegistries::new(),
        &ParamValues::new(),
        None,
    )
    .unwrap_or_else(|f| panic!("{f:#?}"));
    // A family with no operator taking a window port is still refused by name.
    let rsi = def(serde_json::json!({
      "strategy_id": "rsi_sweep", "definition_version": "1.2", "asset_class": "crypto_spot_cex",
      "inputs": [], "parameters": {"n": {"type": "int", "default": 14, "min": 5, "max": 30}},
      "nodes": [{"id": "c", "type": "condition", "expr": "feature('rsi_{{n}}') < 30"},
                {"id": "s", "type": "signal", "when": "c", "emit": "long"}],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "fixed", "size": "0.1"}}]
    }));
    assert_eq!(translate(&rsi, "1h").unwrap_err().construct, "parameterised feature name");
}

/// U-7.3: a decision model's class map translates — each class it may
/// predict emits its signal when the model names it with enough confidence.
#[test]
fn a_decision_models_class_map_becomes_one_condition_per_class() {
    use domain::model_def::contract::{ContractOutput, CostClass, IoContract, PitSpec};
    use domain::strategy_graph::ValueType;
    use strategy_engine::ResolvedModel;
    let d = def(serde_json::json!({
      "strategy_id": "dec", "definition_version": "1.1", "asset_class": "crypto_spot_cex",
      "inputs": [],
      "nodes": [
        {"id": "d", "type": "decision", "model_ref": "trader", "min_confidence": 0.6,
         "class_map": {"long": "go_long", "flat": "go_flat"}}
      ],
      "actions": [
        {"on_signal": "go_long", "type": "place_order",
         "order": {"side": "buy", "size_mode": "fixed", "size": "1"}},
        {"on_signal": "go_flat", "type": "place_order",
         "order": {"side": "sell", "size_mode": "fixed", "size": "1"}}
      ]
    }));
    let t = translate(&d, "1h").unwrap_or_else(|r| panic!("{r}"));
    assert!(t.text.contains("is(d__model.action, \"long\")"), "{}", t.text);
    assert!(t.text.contains("d__model.confidence >= 0.6"), "{}", t.text);
    let reg = StdRegistries::new().with_model(
        "trader@production",
        ResolvedModel {
            model: "trader".into(),
            version: 3,
            contract: IoContract {
                inputs: vec![],
                outputs: vec![
                    ContractOutput {
                        name: "action".into(),
                        value_type: ValueType::Cat {
                            labels: vec!["long".into(), "flat".into()],
                        },
                    },
                    ContractOutput {
                        name: "confidence".into(),
                        value_type: ValueType::Num,
                    },
                ],
                horizon: None,
                pit: PitSpec::default(),
                cost_class: CostClass::Cheap,
                timeout_ms: 1_000,
            },
        },
    );
    validate(Source::Graph(t.graph), &reg, &ParamValues::new(), None)
        .unwrap_or_else(|f| panic!("the translation compiles: {f:#?}"));
}

#[test]
fn constructs_without_a_translation_are_refused_by_name() {
    let base = |extra: serde_json::Value| {
        let mut v = serde_json::json!({
          "strategy_id": "s", "definition_version": "1.1", "asset_class": "crypto_spot_cex",
          "inputs": [], "nodes": [], "actions": []
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        def(v)
    };
    let r = translate(
        &base(serde_json::json!({"risk_overrides": {"max_position": "2"}})),
        "1h",
    )
    .unwrap_err();
    assert_eq!(r.construct, "risk_overrides.max_position");
    assert!(r.why.contains("loosen"));
    let r = translate(&base(serde_json::json!({"nodes": [{"id": "d", "type": "decision", "model_ref": "m", "class_map": {}, "abstain": "hold_last"}]})), "1h").unwrap_err();
    assert_eq!(r.construct, "decision.abstain hold_last");
    let r = translate(
        &base(serde_json::json!({"inputs": [{"lane": "market.bars.1m", "instrument": "ETH-USD"}]})),
        "1h",
    )
    .unwrap_err();
    assert_eq!(r.construct, "inputs");
    // A parameter inside a feature name with no operator to carry it: v1
    // substituted it per value; a v3 feature name is fixed, so this is refused
    // rather than carried broken. (`ema_{{p}}` translates: see below.)
    let r = translate(&base(serde_json::json!({"nodes": [{"id": "c", "type": "condition", "expr": "feature('rsi_{{n}}') > 1"}]})), "1h").unwrap_err();
    assert_eq!(r.construct, "parameterised feature name");
}

#[test]
fn model_outputs_read_as_features_become_fields() {
    let d = def(serde_json::json!({
      "strategy_id": "m", "definition_version": "1.1", "asset_class": "crypto_spot_cex", "inputs": [],
      "nodes": [
        {"id": "inf", "type": "inference", "model_ref": "fc", "outputs": [{"field": "sigma", "as": "vol"}]},
        {"id": "calm", "type": "condition", "expr": "feature('vol') < 0.03"},
        {"id": "s", "type": "signal", "when": "calm", "emit": "go"}
      ],
      "actions": [{"on_signal": "go", "type": "place_order", "order": {"side": "buy", "size_mode": "fixed", "size": "1"}}]
    }));
    let t = translate(&d, "1h").unwrap();
    assert!(
        t.text.contains("let inf = model(\"fc@production\")"),
        "{}",
        t.text
    );
    assert!(t.text.contains("let calm = inf.sigma < 0.03"), "{}", t.text);
    assert!(t.changes.iter().any(|c| c.code == "models_called"));
}

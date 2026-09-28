//! The round trip: `compile(parse(decompile(d))) == d` (S-5.7; AGENT-011 §8.3).
//!
//! # What this is for
//!
//! It is tempting to read this as a test of the decompiler. It is not — it is
//! the thing that makes the text and the JSON **one object**. Two
//! representations of a strategy that are merely *related* drift: someone adds a
//! clause to the language and forgets the reader, or adds a field to the schema
//! and forgets the writer, and six months later `explain_strategy` describes a
//! strategy the simulator does not run. An equality you can execute is the only
//! version of "they are the same thing" that survives being maintained.
//!
//! # Two corpora, for two different reasons
//!
//! `CORPUS` compiles a corpus of SLv2 documents and round-trips the result.
//! It proves the property over everything the language can currently say, and it
//! is easy to extend — add a document, get a case.
//!
//! `HAND_BUILT` starts from JSON nobody in this crate wrote as text. Without
//! it the whole exercise is circular: a compiler and a decompiler that agreed
//! only on the compiler's own output would pass the text corpus while being
//! wrong about every definition that came from the MCP builder.
//!
//! # And the fixpoint
//!
//! Each case also asserts `decompile(d) == decompile(compile(parse(decompile(d))))`
//! — the *text* is stable, not just the definition. A decompiler that emitted a
//! different but equivalent rendering on the second pass would satisfy the
//! equality above and still churn every stored strategy's readable form on every
//! read.

use std::collections::BTreeSet;

use domain::strategy_def::{
    actions::{ActionKind, SizeMode},
    exits::ExitPolicy,
    nodes::NodeKind,
    params::ParamSpec,
    StrategyDefinition,
};
use strategy_lang::form::FormStrategy;
use strategy_lang::skeleton::{worked, HypothesisFamily};
use strategy_lang::{compile, decompile, parse};

/// Compile text into the canonical definition, failing loudly.
fn definition_of(source: &str) -> StrategyDefinition {
    let ast = parse(source).unwrap_or_else(|e| panic!("parse failed for:\n{source}\n{e:#?}"));
    compile(&ast).unwrap_or_else(|e| panic!("compile failed for:\n{source}\n{e:#?}"))
}

/// `compile(parse(decompile(d))) == d`, plus the text fixpoint.
fn assert_round_trips(label: &str, def: &StrategyDefinition) {
    let text = decompile(def).unwrap_or_else(|d| panic!("{label}: decompile refused: {d:#?}"));
    let ast = parse(&text).unwrap_or_else(|e| {
        panic!("{label}: the decompiler wrote text it cannot parse:\n{text}\n{e:#?}")
    });
    let again = compile(&ast).unwrap_or_else(|e| {
        panic!("{label}: the decompiler wrote text it cannot compile:\n{text}\n{e:#?}")
    });
    assert_eq!(
        &again, def,
        "{label}: the definition changed\n--- text ---\n{text}"
    );

    let text_again =
        decompile(&again).unwrap_or_else(|d| panic!("{label}: second decompile refused: {d:#?}"));
    assert_eq!(text, text_again, "{label}: the text is not a fixpoint");
}

/// Everything about a definition the corpus needs to have exercised.
///
/// Collected from the compiled definitions rather than asserted per-document,
/// so that deleting a document from the corpus fails the coverage guard instead
/// of silently shrinking what is proven.
fn shapes(def: &StrategyDefinition, into: &mut BTreeSet<String>) {
    into.insert(format!("version:{}", def.definition_version));
    into.insert(format!("asset_class:{}", def.asset_class));
    into.insert(format!(
        "trust:{}",
        serde_json::to_value(def.min_trust_tier)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    ));
    match &def.exits {
        None => {
            into.insert("exits:absent".into());
        }
        Some(ExitPolicy::RiskLayerOnly) => {
            into.insert("exits:risk_layer_only".into());
        }
        Some(ExitPolicy::Rules { rules }) => {
            for r in rules {
                into.insert(format!("exit_rule:{}", r.kind()));
                // The unit is a shape of its own. A fraction-of-entry distance
                // and a `1.5 atr(14)` distance are written differently, read
                // back differently and are legal at different definition
                // versions, so a corpus that lost the scaled ones would keep
                // passing while the decompiler wrote `%` over an ATR.
                into.insert(format!("exit_unit:{}", r.unit().as_str()));
            }
        }
    }
    for a in &def.actions {
        let ActionKind::PlaceOrder { order } = &a.kind;
        into.insert(format!(
            "size_mode:{}",
            match order.size_mode {
                SizeMode::Fixed => "fixed",
                SizeMode::PercentOfBalance => "percent_of_balance",
                SizeMode::RiskUnit => "risk_unit",
                SizeMode::Model { .. } => "model",
            }
        ));
        into.insert(format!("side:{:?}", order.side).to_lowercase());
    }
    for spec in def.parameters.values() {
        into.insert(format!(
            "param:{}",
            match spec {
                ParamSpec::Int { .. } => "int",
                ParamSpec::Float { .. } => "float",
                ParamSpec::Enum { .. } => "enum",
            }
        ));
    }
    if !def.constraints.is_empty() {
        into.insert("constraints".into());
    }
    let risk = serde_json::to_value(&def.risk_overrides).unwrap_or(serde_json::Value::Null);
    for key in [
        "max_position",
        "max_order_rate_per_minute",
        "max_order_rate_per_second",
    ] {
        if risk.get(key).is_some() {
            into.insert(format!("risk:{key}"));
        }
    }
    for node in &def.nodes {
        if let NodeKind::Condition { expr } = &node.kind {
            // The three shapes of the boolean encoding — see `compile`'s module
            // docs. Matched on the compiled text because that is what the
            // decompiler has to read back.
            if expr.contains(" * ") {
                into.insert("bool:and".into());
            }
            if expr.contains(" + ") && expr.contains("> 0)") {
                into.insert("bool:or".into());
            }
            if expr.contains("(1 - ") {
                into.insert("bool:not".into());
            }
            if expr.contains("param('") {
                into.insert("expr:param".into());
            }
            if expr.contains("{{") {
                into.insert("expr:template".into());
            }
            if expr.contains("bar('") {
                into.insert("expr:bar".into());
            }
        }
    }
    if def
        .inputs
        .iter()
        .any(|i| i.features.iter().any(|f| f == "frobnicate_12"))
    {
        into.insert("feature:unknown_to_the_catalogue".into());
    }
    if def.inputs.iter().any(|i| i.instrument != "$bound_at_init") {
        into.insert("instrument:hardcoded".into());
    }
    // The platform's own worked example for a hypothesis family (S-5.12). It is
    // a shape like any other: a skeleton dropped from the corpus stops being
    // proven to round-trip, and the guard below says so by name.
    if let Some(family) = def.strategy_id.strip_suffix("_skeleton") {
        into.insert(format!("skeleton:{family}"));
    }
}

/// Every shape the corpus must cover for this test to mean what it claims.
const REQUIRED_SHAPES: &[&str] = &[
    "exit_rule:stop_loss",
    "exit_rule:take_profit",
    "exit_rule:trailing_stop",
    "exit_rule:max_holding_bars",
    "exit_rule:on_signal",
    "exits:risk_layer_only",
    "exits:absent",
    "exit_unit:fraction_of_entry",
    "exit_unit:atr",
    "exit_unit:sigma",
    "size_mode:fixed",
    "size_mode:percent_of_balance",
    "size_mode:risk_unit",
    "side:buy",
    "side:sell",
    "param:int",
    "param:float",
    "param:enum",
    "constraints",
    "risk:max_position",
    "risk:max_order_rate_per_minute",
    "risk:max_order_rate_per_second",
    "bool:and",
    "bool:or",
    "bool:not",
    "expr:template",
    "expr:bar",
    "version:1.0",
    "version:1.3",
    "asset_class:equity",
    "trust:regulated",
    "feature:unknown_to_the_catalogue",
    "instrument:hardcoded",
    "expr:param",
    "skeleton:trend",
    "skeleton:mean_reversion",
    "skeleton:breakout",
    "skeleton:vol_target",
];

const CORPUS: &[(&str, &str)] = &[
    (
        "trend",
        "\
strategy ema_cross on $bound_at_init timeframe 1h
input fast = ema(close, 7)
input slow = ema(close, 21)
entry long when fast > slow and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
exit time 48 bars
risk max_position 0.5
",
    ),
    (
        "mean reversion",
        "\
strategy mean_rev on $bound_at_init timeframe 15m
entry long when between(zscore(close, 20), -3, -1)
size equity_fraction 0.25
exit target 1.5 % from entry
exit time 12 bars
risk max_orders_per_min 6
risk max_orders_per_sec 2
",
    ),
    (
        "breakout, both sides",
        "\
strategy breakout on 'BTC-USD' timeframe 1h
entry long when close > donchian_upper(close, 20)
entry short when close < donchian_lower(close, 20)
size risk_per_trade 0.01
exit trailing 3 % from high_water
",
    ),
    (
        "swept parameters",
        "\
strategy swept on $bound_at_init timeframe 1h
param fast: int[5..50 step 5] = 10
param gate: float[0.005..0.08 log] = 0.02
param mode: enum[trail, fixed] = trail
param slow: int[10..200] = 50
constraint param('fast') < param('slow')
input fast_ma = ema(close, fast)
input slow_ma = ema(close, slow)
entry long when fast_ma > slow_ma
size fixed 0.01
exit none
",
    ),
    (
        "a gate written with or and not",
        "\
strategy gated on $bound_at_init timeframe 4h
entry long when not (rsi(close, 14) > 70 or volume < 100) and close > 1
size fixed 0.02
exit stop 4 % from entry
",
    ),
    (
        "a signal exit beside a price exit",
        "\
strategy exiter on $bound_at_init timeframe 1h
entry long when macd(close, 12, 26) > 0
size fixed 0.5
exit trailing 3 % from high_water
exit when rsi(close, 14) > 70
",
    ),
    (
        "a pre-1.3 document on another asset class",
        "\
strategy legacy on 'ETH-USD' timeframe 5m
version 1.0
asset_class equity
trust regulated
entry long when close > 1
size fixed 1
",
    ),
    (
        "a feature this build has never heard of",
        "\
strategy raw on $bound_at_init timeframe 1h
entry long when feature('frobnicate_12') > 0
size fixed 1
exit none
",
    ),
    (
        "an arithmetic comparison between two bar fields",
        "\
strategy spread on $bound_at_init timeframe 1m
entry long when high - low > close * 0.01
size fixed 1
exit time 3 bars
",
    ),
    (
        // Both scaled units in one document, and no `version` header: a scaled
        // distance is what lifts a document to 1.4, so the compiler stamps it
        // rather than the author claiming it. Written with two different
        // lookbacks because the period is part of the unit — `atr(14)` and
        // `atr(50)` are different distances, and a decompiler that dropped the
        // number would still round-trip a corpus that only ever used one.
        "volatility-scaled exits, both units",
        "\
strategy vol_scaled on $bound_at_init timeframe 1h
input trend = ema(close, 20)
entry long when close > trend
size risk_per_trade 0.01
exit stop 1.5 atr(14) from entry
exit target 2.5 sigma(20) from entry
exit trailing 3 atr(50) from high_water
",
    ),
];

#[test]
fn every_document_in_the_corpus_round_trips() {
    let mut covered = BTreeSet::new();
    for (label, source) in CORPUS {
        let def = definition_of(source);
        shapes(&def, &mut covered);
        assert_round_trips(label, &def);
    }
    assert_eq!(
        CORPUS.len(),
        10,
        "the corpus shrank; every document is a case"
    );

    // The skeletons are corpus documents too, and they are the ones a model
    // reads as the shape to imitate. A worked example that did not survive the
    // round trip would be a worked example the platform cannot read back.
    let skeletons = worked();
    assert_eq!(
        skeletons.len(),
        4,
        "four families are writable with the engine's catalogue"
    );
    for s in &skeletons {
        let def = definition_of(s.text);
        shapes(&def, &mut covered);
        assert_round_trips(&format!("skeleton:{}", s.family), &def);
    }

    let missing: Vec<&&str> = REQUIRED_SHAPES
        .iter()
        .filter(|s| !covered.contains(**s))
        .collect();
    assert!(
        missing.is_empty(),
        "the corpus no longer exercises {missing:?}; covered: {covered:?}"
    );
}

/// The non-circular half: definitions this crate never wrote as text.
///
/// Each is canonical — generated node identifiers, generated exit-signal names
/// — because those are what [`strategy_lang::compile`] produces and what the
/// round trip is therefore stated over. `decompile`'s own tests cover what
/// happens to a definition that is not.
const HAND_BUILT: &[(&str, &str)] = &[
    (
        "a minimal definition with no features at all",
        r#"{
          "strategy_id": "bare",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "centralized_exchange",
          "inputs": [{ "lane": "market.bars.1h", "instrument": "$bound_at_init" }],
          "nodes": [
            { "id": "e0_cond", "type": "condition", "expr": "(bar('close') > 0)" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
          ],
          "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
          ],
          "exits": { "mode": "risk_layer_only" }
        }"#,
    ),
    (
        "a short strategy with every price exit",
        r#"{
          "strategy_id": "shorty",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "onchain_confirmed",
          "inputs": [
            { "lane": "market.bars.4h", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["atr_14", "rsi_7"] }
          ],
          "nodes": [
            { "id": "e0_cond", "type": "condition",
              "expr": "((feature('rsi_7') > 70) * (feature('atr_14') > 1))" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "short" }
          ],
          "actions": [
            { "on_signal": "short", "type": "place_order",
              "order": { "side": "sell", "size_mode": "percent_of_balance", "size": "0.1" } }
          ],
          "risk_overrides": { "max_position": "0.25", "max_order_rate_per_minute": 4 },
          "exits": { "mode": "rules", "rules": [
            { "rule": "stop_loss", "fraction": "0.015" },
            { "rule": "take_profit", "fraction": "0.03" },
            { "rule": "trailing_stop", "fraction": "0.008" },
            { "rule": "max_holding_bars", "bars": 6 }
          ]}
        }"#,
    ),
    (
        "a definition whose only exit is a signal",
        r#"{
          "strategy_id": "signal_exit",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "centralized_exchange",
          "inputs": [
            { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_50", "ema_9"] }
          ],
          "nodes": [
            { "id": "e0_cond", "type": "condition",
              "expr": "(feature('ema_9') > feature('ema_50'))" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" },
            { "id": "x0_cond", "type": "condition",
              "expr": "(feature('ema_9') < feature('ema_50'))" },
            { "id": "x0_sig", "type": "signal", "when": "x0_cond", "emit": "x0" }
          ],
          "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "risk_unit", "size": "1" } }
          ],
          "exits": { "mode": "rules", "rules": [{ "rule": "on_signal", "emit": "x0" }] }
        }"#,
    ),
];

#[test]
fn hand_built_definitions_round_trip_too() {
    let mut covered = BTreeSet::new();
    for (label, json) in HAND_BUILT {
        let def: StrategyDefinition = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("{label}: the fixture is not a definition: {e}"));
        shapes(&def, &mut covered);
        assert_round_trips(label, &def);
    }
    assert_eq!(HAND_BUILT.len(), 3, "the corpus shrank");
    // The point of this corpus is the shapes the text one cannot reach from
    // here: a definition with no feature lane at all, and a feature read twice
    // that the decompiler must give a name.
    for shape in ["exits:risk_layer_only", "side:sell", "exit_rule:on_signal"] {
        assert!(covered.contains(shape), "hand-built corpus missed {shape}");
    }
}

/// A feature read more than once must come back as an `input`, because the
/// parser refuses the same call written twice. If this stopped being true the
/// decompiler would write text its own parser rejects — the failure mode the
/// fixpoint assertion exists to catch, pinned here on its own so the reason is
/// legible when it breaks.
#[test]
fn a_feature_read_twice_comes_back_as_a_named_input() {
    let json = r#"{
      "strategy_id": "twice",
      "definition_version": "1.3",
      "asset_class": "crypto_spot_cex",
      "min_trust_tier": "centralized_exchange",
      "inputs": [
        { "lane": "market.bars.1h", "instrument": "$bound_at_init" },
        { "lane": "features.technical", "instrument": "$bound_at_init",
          "features": ["zscore_20"] }
      ],
      "nodes": [
        { "id": "e0_cond", "type": "condition",
          "expr": "((feature('zscore_20') >= 0) * (feature('zscore_20') <= 2))" },
        { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
      ],
      "actions": [
        { "on_signal": "long", "type": "place_order",
          "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }
      ],
      "exits": { "mode": "risk_layer_only" }
    }"#;
    let def: StrategyDefinition = serde_json::from_str(json).expect("the fixture is a definition");
    let text = decompile(&def).expect("decompiles");
    assert!(
        text.contains("input zscore_20 = zscore(close, 20)"),
        "no binding in:\n{text}"
    );
    assert_round_trips("twice", &def);
}

/// A named input whose feature is read once is inlined, so the author's name
/// for it does not survive. That is a real and deliberate loss — the canonical
/// form stores the feature, not the nickname — and stating it here stops a
/// future reader filing it as a bug.
#[test]
fn an_input_name_used_once_does_not_survive_the_round_trip_and_the_definition_does() {
    let source = "\
strategy nick on $bound_at_init timeframe 1h
input my_favourite_indicator = rsi(close, 14)
entry long when my_favourite_indicator < 30
size fixed 1
exit none
";
    let def = definition_of(source);
    let text = decompile(&def).expect("decompiles");
    assert!(
        !text.contains("my_favourite_indicator"),
        "the nickname is not stored anywhere, so it cannot come back:\n{text}"
    );
    assert!(
        text.contains("entry long when rsi(close, 14) < 30"),
        "{text}"
    );
    assert_round_trips("nick", &def);
}

/// `order market` is the one order clause the canonical form can hold, and it
/// holds it by holding nothing: FEAT-004 §3.1 names market as the default and
/// `OrderSpec` has no order-type field, so the clause compiles to the same
/// definition as its absence and cannot come back out of one.
///
/// Recorded as a test rather than left to be discovered, because a clause that
/// is accepted and then invisible on the next read is the shape this file's
/// corpus guard exists to catch — and here it is the honest answer rather than a
/// dropped clause: there is nothing in the definition for `decompile` to have
/// seen. The three types that would mean something (`limit`, `stop`,
/// `stop_limit`) are refused where they are written instead.
#[test]
fn order_market_compiles_to_nothing_and_so_cannot_come_back() {
    let with_clause = "strategy plain on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
order market
exit time 3 bars
";
    let without = "strategy plain on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit time 3 bars
";
    let def = definition_of(with_clause);
    assert_eq!(
        def,
        definition_of(without),
        "`order market` must not change the definition; it is the default said out loud"
    );
    let text = decompile(&def).expect("decompiles");
    assert!(
        !text.contains("order"),
        "nothing in the definition records the clause, so emitting one would be an invention:
{text}"
    );
    assert_round_trips("plain", &def);
}

/// Exit rule order is semantic (`exits.rs`: "a policy whose stop is listed after
/// its target behaves differently"), so the round trip must not sort it.
#[test]
fn exit_rule_order_survives() {
    let a = definition_of(
        "\
strategy order_a on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit target 5 % from entry
exit stop 2 % from entry
",
    );
    let b = definition_of(
        "\
strategy order_b on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 2 % from entry
exit target 5 % from entry
",
    );
    let (Some(ExitPolicy::Rules { rules: ra }), Some(ExitPolicy::Rules { rules: rb })) =
        (&a.exits, &b.exits)
    else {
        panic!("both fixtures declare rules");
    };
    assert_eq!(ra[0].kind(), "take_profit");
    assert_eq!(rb[0].kind(), "stop_loss");
    assert_round_trips("order_a", &a);
    assert_round_trips("order_b", &b);
    assert_ne!(
        decompile(&a).unwrap(),
        decompile(&b).unwrap(),
        "two policies that behave differently must read differently"
    );
}

/// Every skeleton is reachable by a `select_fill` tier, not only by a model
/// that can write text (S-5.9 + S-5.12 together).
///
/// This is the acceptance the two tasks share and neither states on its own: a
/// worked example a small tier cannot express through the clause grammar is a
/// worked example for somebody else. Each skeleton is parsed, projected into
/// the form, rendered back out of it, and compiled — and the definition that
/// comes out is the one the skeleton's own text compiles to.
#[test]
fn every_skeleton_is_reachable_through_the_clause_grammar() {
    let skeletons = worked();
    assert_eq!(
        skeletons.len(),
        4,
        "the set shrank; every skeleton is a case"
    );
    for s in &skeletons {
        let ast = parse(s.text).unwrap_or_else(|e| panic!("{}: {e:#?}", s.family));
        let form = FormStrategy::from_strategy(&ast).unwrap_or_else(|refusals| {
            panic!(
                "the {} skeleton cannot be written through the form: {:?}",
                s.family,
                refusals
                    .iter()
                    .map(|r| (r.limitation, r.diagnostic.message.clone()))
                    .collect::<Vec<_>>()
            )
        });
        assert_eq!(
            form.compile()
                .unwrap_or_else(|e| panic!("{}: {e:#?}", s.family)),
            s.definition()
                .unwrap_or_else(|e| panic!("{}: {e:#?}", s.family)),
            "the {} skeleton means something different through the form",
            s.family
        );
    }
    // And the two families that have no skeleton have none through the form
    // either: there is one answer to "what can I start from", not two.
    for family in [HypothesisFamily::Carry, HypothesisFamily::ModelSignal] {
        assert!(
            !skeletons.iter().any(|s| s.family == family),
            "{family} has a skeleton and the catalogue says it cannot"
        );
    }
}

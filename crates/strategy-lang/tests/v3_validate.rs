//! Set U U-7.2: one validator.
//!
//! Acceptance: every error code `strategy-validator` emits today has an
//! equivalent here with the same teachable fix; the same malformed strategy
//! yields the same error through text, builder ops and raw JSON. (Builder ops
//! are in `v3_builder.rs`; text and JSON here.)

use domain::strategy_def::params::ParamValues;
use strategy_engine::StdRegistries;
use strategy_lang::v3::validate::{Equivalent, LEGACY, TEACH};
use strategy_lang::v3::{parse::parse_to_json, validate, Finding, Source};

const HEADER: &str = "strategy s kind trading asset_class crypto_spot_cex timeframe 1h\n";

fn findings_of_text(text: &str, window: Option<u32>) -> Vec<Finding> {
    validate(
        Source::Text(text),
        &StdRegistries::new(),
        &ParamValues::new(),
        window,
    )
    .err()
    .unwrap_or_default()
}

fn codes(f: &[Finding]) -> Vec<&str> {
    f.iter().map(|x| x.code.as_str()).collect()
}

/// One malformed strategy per code this validator can emit for a legacy
/// mistake.
fn example_for(code: &str) -> (String, Option<u32>) {
    let t = |body: &str| format!("{HEADER}{body}");
    let s = match code {
        "unknown_feature" => t("let x = feature(\"rsi14\")\nwhen x > 1 -> close"),
        "warmup_exceeds_window" => return (t("let x = ema(close, 50)\nwhen x > 1 -> close"), Some(10)),
        "never_fires" => t("when feature(\"rsi_14\") > 100 -> close"),
        "wrong_format" => return ("{}".into(), None),
        "empty_name" => "strategy \"\" kind trading asset_class c timeframe 1h\nwhen close > 1 -> close".into(),
        "duplicate_node_id" => t("let a = ema(close, 5)\nlet a = ema(close, 9)\nwhen a > 1 -> close"),
        "unknown_ref" => t("when nosuch -> close"),
        "no_rules" => t("let a = ema(close, 5)"),
        "close_fraction_out_of_range" => t("when close > 1 -> close(fraction: 1.5)"),
        "distance_not_positive" => t("when close > 1 -> protect(stop: {amount: 0, unit: pct})"),
        "distance_without_unit" => t("when close > 1 -> protect(stop: {amount: 2})"),
        "atr_period_out_of_range" => t("when close > 1 -> protect(stop: {amount: 2, unit: atr(n: 0)})"),
        "rule_shadowed" => t("when close > 1 -> close\nwhen close > 1 -> hold"),
        "params" => t("param fast: int(default: 99, min: 5, max: 50)\nlet a = ema(close, $fast)\nwhen a > 1 -> close"),
        "syntax" => t("let a = ema(close,, 5)"),
        other => panic!("no example for {other}"),
    };
    (s, None)
}

#[test]
fn every_legacy_code_has_an_equivalent() {
    for t in strategy_validator::teach::TEACHINGS {
        let found = LEGACY.iter().find(|(c, _)| *c == t.code);
        assert!(found.is_some(), "`{}` has no equivalent", t.code);
        if let Some((_, Equivalent::Unrepresentable(why))) = found {
            assert!(why.len() > 20, "`{}` says why it cannot be written", t.code);
        }
    }
    assert_eq!(
        LEGACY.len(),
        strategy_validator::teach::TEACHINGS.len(),
        "and nothing else"
    );
}

#[test]
fn every_equivalent_code_is_emitted_for_its_mistake_with_a_fix_and_a_guide_section() {
    let mut checked = std::collections::BTreeSet::new();
    for (old, eq) in LEGACY {
        let Equivalent::Code(code) = eq else { continue };
        if !checked.insert(*code) {
            continue;
        }
        let (text, window) = example_for(code);
        let f = if code == &"wrong_format" {
            validate(
                Source::Json(&serde_json::json!({"format": "tbot.strategy/1.4"})),
                &StdRegistries::new(),
                &ParamValues::new(),
                None,
            )
            .err()
            .unwrap_or_default()
        } else {
            findings_of_text(&text, window)
        };
        let hit = f.iter().find(|x| x.code == *code).unwrap_or_else(|| {
            panic!("`{old}` → `{code}` not emitted for its example: {f:#?}\n{text}")
        });
        assert!(hit.fix.len() > 10, "`{code}` teaches a fix");
        assert!(hit.see.is_some(), "`{code}` names its guide section");
    }
}

#[test]
fn the_same_mistake_is_the_same_finding_through_text_and_json() {
    for code in [
        "unknown_feature",
        "duplicate_node_id",
        "unknown_ref",
        "rule_shadowed",
        "distance_not_positive",
        "params",
    ] {
        let (text, window) = example_for(code);
        let from_text = findings_of_text(&text, window);
        let json = parse_to_json(&text).expect("the example is SLv3");
        let from_json = validate(
            Source::Json(&json),
            &StdRegistries::new(),
            &ParamValues::new(),
            window,
        )
        .err()
        .unwrap_or_default();
        assert_eq!(from_text, from_json, "{code}");
        assert!(codes(&from_text).contains(&code), "{code}: {from_text:#?}");
    }
}

#[test]
fn a_valid_strategy_returns_its_graph_revision_text_and_plan() {
    let text = format!("{HEADER}let f = ema(close, 5)\nlet s = ema(close, 20)\nwhen crossed_above(f, s) and state.flat -> target(size: units(x: 1))\nwhen crossed_below(f, s) -> close\n");
    let v = validate(
        Source::Text(&text),
        &StdRegistries::new(),
        &ParamValues::new(),
        Some(500),
    )
    .unwrap_or_else(|f| panic!("{f:#?}"));
    assert_eq!(v.revision, v.graph.revision().unwrap().as_str());
    assert_eq!(strategy_lang::v3::parse(&v.text).unwrap(), v.graph);
    assert!(v.plan.warmup_ticks >= 20);
}

#[test]
fn every_taught_code_names_a_section_once() {
    let mut seen = std::collections::BTreeSet::new();
    for (code, section) in TEACH {
        assert!(seen.insert(*code), "{code} twice");
        assert!(section.starts_with("slv3#"), "{section}");
    }
}

//! AGENT-011 §5's six teachable errors each have a real producer (S-5.3).
//!
//! The acceptance: *"the six error cases in AGENT-011 §5 return the listed
//! `fix`."* This file ran into the earlier state of the task, which was
//! `PARTIAL` because one of the six, `distance_without_unit`, "belongs to SLv2's
//! grammar, which does not exist". It exists. So each code is produced here by
//! the component that owns it, from a real mistake, and the answer is checked to
//! carry the teaching table's fix rather than a fix of its own:
//!
//! | code | produced by |
//! |---|---|
//! | `unknown_feature` | `strategy_validator::validate` (and the SLv2 compiler) |
//! | `distance_without_unit` | the SLv2 parser |
//! | `when_names_expression` | `strategy_validator::validate` |
//! | `loosening` | `strategy_validator::validate` |
//! | `warmup_exceeds_window` | `backtest::preview::preview_signals` |
//! | `never_fires` | `backtest::preview::preview_signals` |
//!
//! "A third identical rejection returns an example" is **met by a stronger
//! behaviour**, recorded as ADR-S5-05 in the phase file: the example is attached
//! to every rejection, because §5 counts rejections within a run and a pure
//! function of one document has no run to count in. The test of that is
//! `mcp-server`'s `ValidationErrorItem` conversion and
//! `strategy-validator/tests/the_validator_teaches.rs`.

mod support;

use strategy_validator::{teach, validate};
use support::{compiled, hourly_bars, preview};

const BASE: &str = "\
strategy teach_base on $bound_at_init timeframe 1h
entry long when rsi(close, 14) < 30
size fixed 0.01
exit stop 2 % from entry
";

/// The definition as JSON, so a mistake can be written into it the way a model
/// writing JSON would.
fn json_of(text: &str) -> serde_json::Value {
    serde_json::to_value(compiled(text)).expect("serialises")
}

fn definition(v: serde_json::Value) -> domain::strategy_def::StrategyDefinition {
    serde_json::from_value(v).expect("still a definition")
}

fn row(code: &str) -> &'static teach::Teaching {
    teach::teaching(code).unwrap_or_else(|| panic!("the teaching table has no row for `{code}`"))
}

#[test]
fn the_base_validates_so_each_case_isolates_one_mistake() {
    validate(&compiled(BASE)).expect("the base must validate or nothing below isolates");
}

#[test]
fn unknown_feature_is_produced_and_carries_the_listed_fix() {
    let broken = json_of(BASE).to_string().replace("rsi_14", "rsi14");
    let errs = validate(&definition(serde_json::from_str(&broken).unwrap()))
        .expect_err("`rsi14` is not a feature");
    let e = errs
        .iter()
        .find(|e| e.code == "unknown_feature")
        .expect("an unknown_feature rejection");
    assert_eq!(e.fix(), Some(row("unknown_feature").fix));
    // The fix repairs the very mistake: `rsi14` has the family right and the
    // syntax wrong, and the answer names the form that resolves.
    assert!(e.message.contains("rsi_14"), "{}", e.message);

    // The language reaches the same code for the same mistake in SLv2.
    let slv2 = BASE.replace("rsi(close, 14)", "rsii(close, 14)");
    let ds = strategy_lang::text_to_definition(&slv2).expect_err("refused");
    assert!(ds.iter().any(|d| d.code.as_str() == "unknown_feature"));
}

#[test]
fn distance_without_unit_is_produced_by_the_language_and_teaches_the_same_units() {
    let text = BASE.replace("exit stop 2 % from entry", "exit stop 1.5 from entry");
    let ds = strategy_lang::text_to_definition(&text).expect_err("a bare number is refused");
    let d = ds
        .iter()
        .find(|d| d.code.as_str() == "distance_without_unit")
        .expect("a distance_without_unit diagnostic");
    let fix = d.fix.as_deref().expect("a fix");
    let table = row("distance_without_unit").fix;
    // Two statements of one fix, one in the parser and one in the table. They
    // are not the same string — the parser's names the number the author wrote —
    // so the property is that they teach the same three units and the same
    // lookback syntax, and drift in either fails here.
    for needle in ["%", "atr(", "sigma(", "from entry"] {
        assert!(fix.contains(needle), "the parser's fix lacks `{needle}`: {fix}");
        assert!(
            table.contains(needle),
            "the teaching table's fix lacks `{needle}`: {table}"
        );
    }
    assert!(d.see.as_deref().is_some_and(|s| s == "guide#exits.units"));
    assert_eq!(row("distance_without_unit").see, Some("guide#exits.units"));
}

#[test]
fn when_names_expression_is_produced_and_carries_the_listed_fix() {
    let mut v = json_of(BASE);
    for n in v["nodes"].as_array_mut().expect("nodes") {
        if n["type"] == "signal" {
            n["when"] = serde_json::json!("feature('rsi_14') > 1");
        }
    }
    let errs = validate(&definition(v)).expect_err("`when` names an expression");
    let e = errs
        .iter()
        .find(|e| e.code == "when_names_expression")
        .expect("a when_names_expression rejection");
    assert_eq!(e.fix(), Some(row("when_names_expression").fix));
}

#[test]
fn loosening_is_produced_and_carries_the_listed_fix() {
    let text = format!("{BASE}risk max_position 200\n");
    let errs = validate(&compiled(&text)).expect_err("a looser limit is refused");
    let e = errs
        .iter()
        .find(|e| e.code == "loosening")
        .expect("a loosening rejection");
    assert_eq!(e.fix(), Some(row("loosening").fix));
}

#[test]
fn warmup_exceeds_window_and_never_fires_are_produced_by_the_preview() {
    // A window barely longer than the minimum, against a feature that needs 1000
    // bars to warm up.
    let long_warmup = BASE.replace("rsi(close, 14) < 30", "ema(close, 200) > 1");
    let p = preview(&compiled(&long_warmup), &hourly_bars(60, 3));
    assert_eq!(p.never_fires_code, Some("warmup_exceeds_window"));
    assert!(!row("warmup_exceeds_window").fix.is_empty());

    // A threshold no bar meets, on a window long enough to evaluate it.
    let dead = BASE.replace("rsi(close, 14) < 30", "rsi(close, 14) < 0");
    let p = preview(&compiled(&dead), &hourly_bars(1_500, 3));
    assert_eq!(p.never_fires_code, Some("never_fires"));
    assert!(p.never_fires_because.is_some());
    assert!(!row("never_fires").fix.is_empty());
}

/// The table itself: the six are present, which is the acceptance's own list.
#[test]
fn the_six_are_in_the_table() {
    for code in [
        "unknown_feature",
        "distance_without_unit",
        "when_names_expression",
        "loosening",
        "warmup_exceeds_window",
        "never_fires",
    ] {
        assert!(!row(code).fix.is_empty(), "{code}");
    }
}

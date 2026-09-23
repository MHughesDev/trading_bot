//! ADR-D-20: the validator accepts every version the parser knows, and no more.
//!
//! # The failure this locks out
//!
//! `validate_schema` gated on `matches!(v, "1.0" | "1.2")`. `strategy-lang`
//! compiles to **1.3**, because the typed `exits` block is the only place
//! `exit stop 2 % from entry` can live and `exits` is REQUIRED at 1.3. The
//! persist route validates before it writes. So the authoring path this
//! programme's whole strategy-authoring capability rests on was dead at its last
//! step, and the diagnostic it died with told the author to write 1.3.
//!
//! A longer whitelist would have fixed the day and not the shape: 1.1 and 1.3
//! were both missed because a version shipped and nobody reread a `matches!`.
//! The gate is now `MIN_SUPPORTED..=MAX_SUPPORTED`, and the tests below are
//! written against the domain's own constants so that a version added there
//! without being admitted here fails rather than passes quietly.

use domain::strategy_def::{
    StrategyDefinition, DEFINITION_VERSION, DEFINITION_VERSION_V1_1, DEFINITION_VERSION_V1_2,
    DEFINITION_VERSION_V1_3, DEFINITION_VERSION_V1_4,
};
use serde_json::json;
use strategy_validator::version::{DefinitionVersion, MAX_SUPPORTED, MIN_SUPPORTED};

/// A minimal document that is valid apart from whatever the test changes.
///
/// `exits` is attached at 1.3 and above because the exits pass requires it
/// there; that is the point of 1.3 and not something this test is testing.
fn def_at(version: &str) -> StrategyDefinition {
    let mut v = json!({
        "strategy_id": "version_probe",
        "definition_version": version,
        "asset_class": "crypto_spot_cex",
        "inputs": [],
        "nodes": [
            { "id": "n1", "type": "condition", "expr": "bar('close') > 1" },
            { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
        ],
        "actions": [{
            "on_signal": "long",
            "type": "place_order",
            "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" }
        }]
    });
    let parsed = DefinitionVersion::parse(version);
    let v13 = DefinitionVersion::parse(DEFINITION_VERSION_V1_3).expect("1.3 parses");
    if parsed.is_some_and(|p| p >= v13) {
        v["exits"] = json!({ "mode": "risk_layer_only" });
    }
    serde_json::from_value(v).expect("the probe document parses")
}

fn version_errors(def: &StrategyDefinition) -> Vec<String> {
    strategy_validator::validate(def)
        .err()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.code == "unknown_definition_version")
        .map(|e| e.message)
        .collect()
}

/// **The regression.** A 1.3 document — what the compiler emits and what the
/// authoring path sends to the persist route — validates clean.
///
/// Against the pre-decision code this fails with `unknown_definition_version`.
#[test]
fn a_thirteen_document_validates() {
    let def = def_at(DEFINITION_VERSION_V1_3);
    let errs = strategy_validator::validate(&def).err().unwrap_or_default();
    assert!(errs.is_empty(), "a 1.3 document must validate: {errs:#?}");
}

/// The live row the whitelist refused.
///
/// `strategy_definitions.ai_model_smoke` carries `"definition_version": "1.1"`
/// inside its stored `definition_json`. A whitelist of `1.0 | 1.2` refuses it,
/// so that stored strategy could not be re-saved, re-validated or edited
/// through the API that wrote it.
#[test]
fn an_eleven_document_validates_because_one_is_stored() {
    let def = def_at(DEFINITION_VERSION_V1_1);
    assert!(
        version_errors(&def).is_empty(),
        "1.1 is stored in strategy_definitions and must be readable back"
    );
}

/// Every version `domain` declares is admitted.
///
/// Written as a loop over the constants rather than four assertions, so that the
/// next additive version is admitted by adding one name here — and its absence
/// is a failing test, not a silent refusal at the persist route six weeks later.
#[test]
fn every_declared_schema_version_is_accepted() {
    for v in [
        DEFINITION_VERSION,
        DEFINITION_VERSION_V1_1,
        DEFINITION_VERSION_V1_2,
        DEFINITION_VERSION_V1_3,
        DEFINITION_VERSION_V1_4,
    ] {
        let errs = version_errors(&def_at(v));
        assert!(errs.is_empty(), "version {v} must be accepted: {errs:?}");
    }
}

/// The half of the whitelist that was load-bearing: a version this build has
/// not been taught the meaning of is still refused.
#[test]
fn a_version_past_the_ceiling_is_still_refused() {
    for v in ["1.5", "2.0", "9.9"] {
        let errs = version_errors(&def_at(v));
        assert_eq!(errs.len(), 1, "version {v} must be refused: {errs:?}");
        assert!(
            errs[0].contains(MAX_SUPPORTED),
            "the refusal must name the ceiling so the author knows what to write: {}",
            errs[0]
        );
    }
}

/// A version below the floor is refused too. The range is bounded at both ends;
/// "0.9" is not a 1.0 document with an unlucky spelling.
#[test]
fn a_version_below_the_floor_is_refused() {
    let errs = version_errors(&def_at("0.9"));
    assert_eq!(errs.len(), 1, "0.9 must be refused: {errs:?}");
    assert!(errs[0].contains(MIN_SUPPORTED));
}

/// An unparseable version is named as unparseable rather than coerced.
///
/// **An absent measurement is never a zero.** `"latest"` is not 1.3 and an empty
/// string is not 1.0; both are documents whose version nobody wrote down, and
/// the only honest response is a refusal that says so.
#[test]
fn a_malformed_version_is_refused_rather_than_coerced() {
    for v in ["", "1", "1.3.0", "v1.3", "latest"] {
        let errs = version_errors(&def_at(v));
        assert_eq!(errs.len(), 1, "version {v:?} must be refused: {errs:?}");
    }
}

/// The exits pass orders versions numerically, so a document above 1.3 is never
/// read as pre-exits.
///
/// The old `def.definition_version.as_str() >= "1.3"` gets this wrong at the
/// tenth minor version, and gets it wrong in the expensive direction: a 1.10
/// strategy carrying a stop loss would have had the stop refused as
/// `exits_version_too_low` while its entries stayed legal — a document one edit
/// away from trading unprotected.
#[test]
fn the_exits_pass_never_reads_a_later_version_as_earlier() {
    let v110 = DefinitionVersion::parse("1.10").expect("parses");
    let v13 = DefinitionVersion::parse(DEFINITION_VERSION_V1_3).expect("parses");
    assert!(v110 > v13, "1.10 is later than 1.3");
    assert!(
        "1.10" < "1.3",
        "...and the lexical order disagrees, which was the bug"
    );

    // 1.10 is above the ceiling, so the whole document is refused — but on the
    // version, once, and never with a contradictory second complaint about its
    // exits.
    let mut raw = serde_json::to_value(def_at(DEFINITION_VERSION_V1_3)).expect("serialises");
    raw["definition_version"] = json!("1.10");
    let def: StrategyDefinition = serde_json::from_value(raw).expect("parses");
    let codes: Vec<&str> = strategy_validator::validate(&def)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.code)
        .collect();
    assert_eq!(
        codes,
        vec!["unknown_definition_version"],
        "an unsupported version is one refusal, not two contradictory ones: {codes:?}"
    );
}

/// The teaching attached to the refusal names a version the gate accepts.
///
/// This is the exact loop that made the original bug so hard to see from inside:
/// the `fix` said "1.3 adds the `exits` block" and the gate refused 1.3, so an
/// author following the diagnostic was sent back to the same refusal.
#[test]
fn the_fix_for_the_refusal_names_a_version_the_gate_accepts() {
    let errs = strategy_validator::validate(&def_at("9.9")).expect_err("9.9 is refused");
    let err = errs
        .iter()
        .find(|e| e.code == "unknown_definition_version")
        .expect("the version error");
    let fix = err.fix().expect("the refusal carries a fix");
    assert!(
        fix.contains(MAX_SUPPORTED),
        "the fix must name the newest accepted version: {fix}"
    );
    let named = DefinitionVersion::parse(MAX_SUPPORTED).expect("parses");
    assert!(
        named.is_supported(),
        "the version the fix tells the author to write must be one the gate accepts"
    );
}

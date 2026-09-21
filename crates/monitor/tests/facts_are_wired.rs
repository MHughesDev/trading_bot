//! COMP-007 §4.1 has a production caller, or says which one it is missing.
//!
//! §4.1 says a reconciliation mismatch, a lost lease and hard staleness "are
//! facts, not estimates, and act immediately". On 2026-09-21 that sentence was
//! true in the type system, true in nine tests, and did nothing whatever in a
//! running platform: no non-test code constructed any of the three facts, and
//! `Evaluator::on_fact` was called by nothing outside tests. Every test on the
//! surface passed. This is the repository's dominant defect shape, on the one
//! surface where it costs money.
//!
//! So the wiring is asserted rather than described. Two properties:
//!
//! 1. Every `Fact` variant either has a production raiser — named, and checked
//!    against the source — or is on a **known-unwired list with a reason and the
//!    exact caller that would clear it**. The list may shrink and may not grow.
//! 2. Every table a `FactAction` writes to is one `LeaseWatcher::refresh` reads,
//!    because a decision written anywhere else never reaches the order gate.
//!
//! Two of the three are unwired today and that is the correct answer: their
//! producers live in crates this track may not edit. A guard that passed because
//! it matched nothing would be worse than no guard, so the scan asserts it found
//! the variants it was written for before it asserts anything about them.

use std::fs;
use std::path::{Path, PathBuf};

use monitor::facts::GateTable;
use monitor::family::{FactKind, Wiring};

/// The variants nothing raises today, and nothing but their producers can.
///
/// Shrinking this list is the work. Growing it is a regression, and the test
/// below refuses it.
const KNOWN_UNWIRED: &[&str] = &["ReconciliationMismatch", "HardDataStaleness"];

/// The variants the scan was written for. If the scan stops finding these, it is
/// broken rather than satisfied.
const EXPECTED_VARIANTS: &[&str] = &[
    "ReconciliationMismatch",
    "LeaseLost",
    "HardDataStaleness",
];

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// A source file with its test module removed.
///
/// Every file in this crate keeps its tests in one trailing `#[cfg(test)] mod
/// tests`, so truncating there is exact. It also matters: `Fact::LeaseLost`
/// appears in nine tests, and a scan that counted those would report the wiring
/// that was missing as present — which is precisely the mistake being guarded
/// against.
fn production_source(relative: &str) -> String {
    let src = fs::read_to_string(crate_root().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"));
    match src.find("#[cfg(test)]") {
        Some(i) => src[..i].to_owned(),
        None => src,
    }
}

/// The body of one function, from its signature to the next item at the same
/// indentation.
fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature:?} is not in this source any more"));
    let rest = &source[start + signature.len()..];
    let end = rest.find("\n    pub ").unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// The `Fact` variant names, read out of the enum rather than assumed.
fn fact_variants_declared() -> Vec<String> {
    let src = production_source("src/family.rs");
    let start = src
        .find("pub enum Fact {")
        .expect("crates/monitor/src/family.rs must still declare `pub enum Fact`");
    let body = &src[start..];
    let end = body.find("\n}").expect("the enum must close");
    body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("///") || line.starts_with("//") {
                return None;
            }
            let name: String = line
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let starts_upper = name.chars().next().is_some_and(char::is_uppercase);
            (starts_upper && name != "Fact").then_some(name)
        })
        .collect()
}

#[test]
fn the_scan_finds_the_fact_variants_it_was_written_for() {
    // The anti-vacuity assertion, and it comes first on purpose. Every claim
    // below is about a variant the scan found; a scan that found none would make
    // all of them true.
    let declared = fact_variants_declared();
    for expected in EXPECTED_VARIANTS {
        assert!(
            declared.iter().any(|d| d.as_str() == *expected),
            "the scan did not find Fact::{expected}; it is broken, not satisfied \
             (found {declared:?})"
        );
    }
    let kinds: Vec<&str> = FactKind::all().iter().map(|k| k.as_str()).collect();
    assert_eq!(
        declared.len(),
        kinds.len(),
        "`Fact` declares {declared:?} but `FactKind::all` lists {kinds:?} — a variant \
         with no kind has no wiring answer either"
    );
    for kind in &kinds {
        assert!(
            declared.iter().any(|d| d.as_str() == *kind),
            "FactKind::{kind} names no Fact variant"
        );
    }
}

#[test]
fn every_fact_variant_is_raised_in_production_or_on_the_known_unwired_list() {
    let mut unwired = Vec::new();

    for kind in FactKind::all() {
        match kind.wiring() {
            Wiring::RaisedBy(site) => {
                // The declaration is checked against the source, because a
                // declaration nobody checks is a comment with a type.
                let (module, function) = site
                    .rsplit_once("::")
                    .unwrap_or_else(|| panic!("{site:?} must be a `::` path"));
                let file = module_file(module);
                let source = production_source(&file);
                assert!(
                    source.contains(&format!("Fact::{}", kind.as_str())),
                    "{site} is declared to raise Fact::{} and {file} does not mention it \
                     outside its tests",
                    kind.as_str()
                );
                assert!(
                    source.contains(&format!("fn {function}")),
                    "{site} does not exist in {file}"
                );
                // And it must be reachable: a raiser nothing calls is the same
                // defect one level down.
                let callers = source.matches(&format!("{function}(")).count();
                assert!(
                    callers > 1,
                    "{site} is declared as the raiser of Fact::{} and nothing in {file} \
                     calls it — a production caller that is itself unreached is not one",
                    kind.as_str()
                );
            }
            Wiring::Unwired { because, needs } => {
                assert!(
                    because.len() > 20,
                    "Fact::{} is unwired for the reason {because:?}, which says nothing",
                    kind.as_str()
                );
                assert!(
                    needs.contains("::"),
                    "Fact::{} must name the exact function that has to raise it; got \
                     {needs:?}",
                    kind.as_str()
                );
                unwired.push(kind.as_str());
            }
        }
    }

    for name in &unwired {
        assert!(
            KNOWN_UNWIRED.contains(name),
            "Fact::{name} has no production caller and is not on the known-unwired list. \
             Either wire it or add it with a reason and the exact caller that would."
        );
    }
    assert!(
        unwired.len() <= KNOWN_UNWIRED.len(),
        "the unwired list may shrink and may not grow: {unwired:?} against {KNOWN_UNWIRED:?}"
    );
}

#[test]
fn the_known_unwired_list_is_not_stale() {
    // The other direction, and the one that rots silently: a variant somebody
    // wired without deleting its entry here leaves a list that describes a
    // problem nobody has any more, which is how a ledger stops being read.
    for name in KNOWN_UNWIRED {
        let kind = FactKind::all()
            .into_iter()
            .find(|k| k.as_str() == *name)
            .unwrap_or_else(|| panic!("the known-unwired list names Fact::{name}, which is gone"));
        assert!(
            !kind.wiring().is_raised_in_production(),
            "Fact::{name} now has a production raiser — take it off KNOWN_UNWIRED"
        );
    }
}

#[test]
fn every_table_a_fact_writes_to_is_one_the_order_gate_is_refreshed_from() {
    // The rule that made both of the monitor track's earlier defects possible:
    // the manual API route and the monitor process update two different things,
    // and anything meant to reach the order gate must travel through a table
    // `LeaseWatcher::refresh` reads — or it passes every test and does nothing.
    //
    // Asserted over the source of `refresh` rather than over its behaviour,
    // because the failure is a read being dropped, and a dropped read leaves
    // every behavioural test green: the mirror simply keeps whatever it had.
    let refresh = function_body(
        &production_source("src/service.rs"),
        "pub async fn refresh(&self) -> LeaseStatus {",
    );
    assert!(
        refresh.len() > 200,
        "the refresh body did not parse; the guard is broken, not satisfied"
    );
    for table in GateTable::all() {
        assert!(
            refresh.contains(table.reader()),
            "LeaseWatcher::refresh no longer reads {} via {}() — every decision that \
             travels through that table now stops at the process boundary",
            table.table(),
            table.reader()
        );
    }
}

/// `monitor::service::LeaseWatcher` → `src/service.rs`.
///
/// The path names a module and a type; only the module segment maps to a file.
fn module_file(path: &str) -> String {
    let module = path
        .split("::")
        .nth(1)
        .unwrap_or_else(|| panic!("{path:?} must be a `monitor::<module>::…` path"));
    format!("src/{module}.rs")
}

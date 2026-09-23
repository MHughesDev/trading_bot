//! `0132_trial_reservation.sql`'s kind CHECK is the catalogue's `TrialKind`
//! (ADR-S10-10).
//!
//! The drift here is the one OQ-20 was: a kind a tool declares and the table
//! refuses is a tool whose every call fails at the door, reported as the tool's
//! own failure. `none` is deliberately absent from both — a reservation exists
//! *because* a tool declared a trial — so this asserts the two lists differ by
//! exactly that.
//!
//! No database needed: the migration is read as text and the enum as code.

use std::path::{Path, PathBuf};

use mcp_server_lib::spec::TrialKind;

fn migration() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0132_trial_reservation.sql");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The SQL with its `--` comments removed — this file's prose names every value.
fn statements(sql: &str) -> String {
    sql.lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn kind_check(sql: &str) -> Vec<String> {
    let at = sql.find("kind IN (").expect("no `kind IN (` CHECK in 0132");
    let rest = &sql[at + "kind IN (".len()..];
    let close = rest.find(')').expect("the list closes");
    rest[..close]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Every kind a tool can declare is a kind the table accepts.
#[test]
fn the_reservable_kinds_are_the_catalogue_kinds_minus_none() {
    let sql = statements(&migration());
    let mut listed = kind_check(&sql);
    listed.sort();

    let mut rust: Vec<String> = [TrialKind::Search, TrialKind::Falsification]
        .iter()
        .map(|k| mcp_server_lib::gate::trial_kind_str(*k).to_string())
        .collect();
    rust.sort();
    assert_eq!(
        listed, rust,
        "0132's kind CHECK and the catalogue's `TrialKind` have drifted. A kind a tool declares \
         and the table refuses is a tool whose every call fails at the door and is reported as \
         the tool's own failure — which is the defect this table was written to end."
    );

    // `none` is in neither, and that is the point rather than an omission.
    assert!(
        !listed.iter().any(|k| k == "none"),
        "`none` is reservable, but a reservation exists because a tool declared a trial"
    );
    assert_eq!(
        mcp_server_lib::gate::trial_kind_str(TrialKind::None),
        "none"
    );
}

/// The Rust-side list the route validates against is the same list.
///
/// Two places check the kind — `ledger::reservation::RESERVABLE_KINDS` before the
/// insert and the CHECK at it — and the looser one would be the real one if they
/// drifted.
#[test]
fn the_ledgers_reservable_kinds_are_the_migrations() {
    let sql = statements(&migration());
    let mut listed = kind_check(&sql);
    listed.sort();

    let mut ledger_side: Vec<String> = ledger::reservation::RESERVABLE_KINDS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    ledger_side.sort();
    assert_eq!(listed, ledger_side);
}

/// Every trial-declaring tool in the catalogue can actually be reserved.
///
/// The end-to-end form of the two above, and the one that would have caught
/// OQ-20 if the table had existed: it walks the real catalogue rather than a
/// list of kinds.
#[test]
fn every_trial_spending_tool_declares_a_reservable_kind() {
    let sql = statements(&migration());
    let listed = kind_check(&sql);
    let mut checked = 0;
    for spec in mcp_server_lib::spec::catalog() {
        if spec.spends_trial == TrialKind::None {
            continue;
        }
        let kind = mcp_server_lib::gate::trial_kind_str(spec.spends_trial);
        assert!(
            listed.iter().any(|k| k == kind),
            "`{}` declares `{kind}` and `mlops.trial_reservation` refuses it, so every call to \
             it would be refused at the door",
            spec.name
        );
        checked += 1;
    }
    assert!(
        checked >= 7,
        "only {checked} trial-spending tools found; the catalogue declared seven when this was \
         written, so the scan is not reading it"
    );
}

/// `0133`'s workflow CHECK is `harness::state::Workflow` (S-10.14).
///
/// Three lists have to agree: the harness enum a run sets, the `ledger` constant
/// the route validates against, and the CHECK at the insert. A workflow the
/// harness can produce and the table refuses is a run whose every
/// trial-spending call fails at the door — the same shape as OQ-20, one column
/// over.
#[test]
fn the_workflow_check_is_the_harness_enum() {
    let sql = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crates/<name>")
            .join("migrations/0133_reservation_workflow.sql"),
    )
    .expect("0133 is readable");
    let sql = statements(&sql);

    let at = sql
        .find("workflow IN (")
        .expect("no `workflow IN (` CHECK in 0133");
    let rest = &sql[at + "workflow IN (".len()..];
    let close = rest.find(')').expect("the list closes");
    let mut listed: Vec<String> = rest[..close]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    listed.sort();

    let mut from_harness: Vec<String> = harness::state::Workflow::ALL
        .iter()
        .map(|w| w.as_str().to_string())
        .collect();
    from_harness.sort();
    assert_eq!(
        listed, from_harness,
        "0133's workflow CHECK and `harness::state::Workflow` have drifted"
    );

    let mut from_ledger: Vec<String> = ledger::reservation::WORKFLOWS
        .iter()
        .map(|w| (*w).to_string())
        .collect();
    from_ledger.sort();
    assert_eq!(
        listed, from_ledger,
        "0133's workflow CHECK and `ledger::reservation::WORKFLOWS` have drifted"
    );
}

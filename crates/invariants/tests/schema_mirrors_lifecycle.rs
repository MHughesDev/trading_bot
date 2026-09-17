//! `0126_run_lifecycle.sql` is a mirror of `harness::state`, and stays one.
//!
//! The same argument `schema_mirrors_memory_types.rs` makes, applied to the four
//! lifecycle enums. Drift here is silent in the worst direction: add a `Stage` in
//! Rust, forget the CHECK, and every run that reaches it fails at the boundary
//! against a constraint nobody was looking at — at whatever hour the lifecycle
//! first reaches that stage, which by definition is a stage nobody has run yet.
//!
//! No database needed. It reads the migration as text and the enums as code,
//! which is what makes it a guard that actually runs: a check that only runs
//! where Postgres does is a check that does not run in CI.

use std::path::{Path, PathBuf};

use harness::lifecycle::evaluate::{Role, Verdict, Why};
use harness::state::{Mode, Ring, Stage, Workflow};

fn migration_named(file: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations")
        .join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn migration() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0126_run_lifecycle.sql");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The SQL with its `--` comments removed.
///
/// A lesson already paid for here: a scan that reads its own prose finds
/// whatever it forbids. The memory mirror's first run failed on a word that
/// appeared only in a comment explaining why there was no such column.
fn statements(sql: &str) -> String {
    sql.lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The quoted strings inside the `IN (...)` that follows `column IN (`.
fn check_list(sql: &str, column: &str) -> Vec<String> {
    let at = sql
        .find(&format!("{column} IN ("))
        .unwrap_or_else(|| panic!("no `{column} IN (` CHECK in 0126"));
    let rest = &sql[at..];
    let open = rest.find("IN (").expect("the list opens") + "IN (".len();
    let rest = &rest[open..];
    let close = rest.find(')').expect("the IN list closes");
    rest[..close]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// How an enum serialises, which is what the column stores.
fn json_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(str::to_string))
        .expect("these enums serialise as strings")
}

#[test]
fn the_mode_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut listed = check_list(&sql, "mode");
    listed.sort();
    let mut rust = vec![json_name(&Mode::Chat), json_name(&Mode::Research)];
    rust.sort();
    assert_eq!(listed, rust);
}

#[test]
fn the_workflow_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut listed = check_list(&sql, "workflow");
    listed.sort();
    let mut rust: Vec<String> = [
        Workflow::Investigate,
        Workflow::Falsify,
        Workflow::Replicate,
        Workflow::Compare,
        Workflow::Diagnose,
        Workflow::Survey,
        Workflow::Optimise,
    ]
    .iter()
    .map(json_name)
    .collect();
    rust.sort();
    assert_eq!(
        listed, rust,
        "a workflow the database refuses is a run that cannot start, and a workflow \
         nothing reads back is a column that lies"
    );
}

#[test]
fn the_ring_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut listed = check_list(&sql, "ring");
    listed.sort();
    let mut rust: Vec<String> = [Ring::Research, Ring::PlatformArtifacts, Ring::Paper]
        .iter()
        .map(json_name)
        .collect();
    rust.sort();
    assert_eq!(listed, rust);
    assert_eq!(
        listed.len(),
        3,
        "three, not four: ring 3 is the one with no door, so there is no value for it \
         and a row cannot claim it"
    );
}

#[test]
fn the_stage_check_is_the_rust_enum() {
    let sql = statements(&migration());
    let mut listed = check_list(&sql, "stage");
    listed.sort();
    let mut rust: Vec<String> = [
        Stage::Explore,
        Stage::Hypothesise,
        Stage::Preregister,
        Stage::Design,
        Stage::Run,
        Stage::Refine,
        Stage::Compare,
        Stage::Reproduce,
        Stage::Propose,
        Stage::Closed,
    ]
    .iter()
    .map(json_name)
    .collect();
    rust.sort();
    assert_eq!(listed, rust);
}

/// A chat turn is ring 0, in the schema as well as in exposure.
///
/// Two mechanisms with different failure modes, on purpose: exposure is a
/// routing property that a future code path could route around, and this is a
/// row that cannot exist.
#[test]
fn a_chat_row_cannot_carry_a_wider_ring() {
    let sql = statements(&migration());
    assert!(
        sql.contains("a_chat_turn_reaches_nothing"),
        "the belt-and-braces constraint is missing"
    );
    assert!(
        sql.contains("mode <> 'chat' OR ring = 'research'"),
        "and it must say what it enforces"
    );
}

/// The defaults are the narrow reading.
///
/// A column that defaulted to `research` mode or `paper` ring would make every
/// row written before the writers were updated claim more than it should. The
/// direction of a wrong default is the whole question here.
#[test]
fn the_defaults_are_the_narrow_reading() {
    let sql = statements(&migration());
    assert!(sql.contains("mode TEXT NOT NULL DEFAULT 'chat'"), "{sql}");
    assert!(sql.contains("ring TEXT NOT NULL DEFAULT 'research'"), "{sql}");
    assert!(
        sql.contains("stage TEXT NOT NULL DEFAULT 'explore'"),
        "{sql}"
    );
    assert!(
        !sql.contains("workflow TEXT NOT NULL"),
        "a chat turn is doing none of the seven, and `investigate` would be a lie \
         rather than a default"
    );
}

// ── 0127: the verdict the platform writes (AGENT-013 §6.2; S-10.4) ──────────

/// The reasons an `inconclusive` may give are the Rust enum's.
///
/// Drift here is the bad kind: add a `Why` in Rust, forget the CHECK, and
/// `evaluate` produces a verdict the database refuses — at whatever hour a run
/// first reaches that reason, which by definition is a situation nobody has hit
/// yet. Widen the CHECK and forget the Rust, and a row exists that nothing can
/// read back.
#[test]
fn the_inconclusive_reasons_are_the_rust_enum() {
    let sql = statements(&migration_named("0127_preregistration.sql"));
    let mut listed = check_list(&sql, "inconclusive_why");
    listed.sort();
    let mut rust: Vec<String> = Why::ALL.iter().map(|w| w.as_str().to_string()).collect();
    rust.sort();
    assert_eq!(listed, rust);
}

/// The statuses are the verdicts, plus `open`.
///
/// `open` has no Rust variant on purpose: it is the absence of a verdict, and a
/// `Verdict::Open` would be a value `evaluate` could return — which is the one
/// thing it must not be able to do quietly.
#[test]
fn the_statuses_are_the_verdicts_plus_open() {
    let sql = statements(&migration_named("0127_preregistration.sql"));
    let mut listed = check_list(&sql, "status");
    listed.sort();
    let mut expected = vec![
        "open".to_string(),
        Verdict::Supported.as_str().to_string(),
        Verdict::Refuted.as_str().to_string(),
        Verdict::Inconclusive {
            why: Why::InsufficientData,
        }
        .as_str()
        .to_string(),
    ];
    expected.sort();
    assert_eq!(listed, expected);
}

/// The roles are the Rust enum's three.
#[test]
fn the_roles_are_the_rust_enum() {
    let sql = statements(&migration_named("0127_preregistration.sql"));
    let mut listed = check_list(&sql, "role");
    listed.sort();
    let mut rust: Vec<String> = [Role::Primary, Role::Secondary, Role::Exploratory]
        .iter()
        .map(|r| r.as_str().to_string())
        .collect();
    rust.sort();
    assert_eq!(listed, rust);
}

/// Only `inconclusive` carries a reason, and it must.
///
/// The pair is the mechanism: "we could not tell" with no reason is the answer a
/// run reaches for when it has nothing, and it is exactly the one that must cost
/// something to give.
#[test]
fn inconclusive_is_the_only_status_with_a_reason_and_it_is_required() {
    let sql = statements(&migration_named("0127_preregistration.sql"));
    assert!(sql.contains("inconclusive_says_why"), "the constraint is missing");
    assert!(
        sql.contains("status <> 'inconclusive' AND inconclusive_why IS NULL"),
        "nothing else may carry a reason"
    );
    assert!(
        sql.contains("status = 'inconclusive' AND inconclusive_why IS NOT NULL"),
        "and an inconclusive must"
    );
}

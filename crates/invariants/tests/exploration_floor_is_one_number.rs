//! The exploration floor is one number, and both halves of it enforce the same
//! one (AGENT-014 §5.2, INV-21; Set S task S-11.1).
//!
//! # The acceptance says "both halves", adversarially
//!
//! AGENT-014 §12.3: "the share cannot be set below 0.10 (the constructor
//! refuses; the column CHECKs)". Two enforcement points, so two ways to get it
//! wrong: a constructor that refuses a value the column would have taken, and —
//! the one that matters — a column that takes a value the constructor refuses,
//! which is the case where a session quietly runs with no exploration at all and
//! only ever confirms the ranking it already had.
//!
//! This test reads the floor from Rust and looks for it in the migration. It is
//! a text check, and that is its limit: it cannot prove the CHECK *fires*, only
//! that the constraint exists and names the same number. Executing it needs a
//! database, and the database half is `an_illegal_session_is_refused` in the
//! ignored suite.
//!
//! What it does catch is the failure that actually happens: somebody lowers the
//! constant, or relaxes the constraint, and the other half stays where it was.

use research_session::charter::{ExplorationShare, EXPLORATION_FLOOR};

const MIGRATION: &str = include_str!("../../../migrations/0138_research_sessions.sql");

#[test]
fn the_migration_checks_the_number_rust_refuses_below() {
    let floor = format!("{EXPLORATION_FLOOR:.2}");
    let constraint = MIGRATION
        .split_once("CONSTRAINT exploration_never_below_the_floor CHECK (")
        .expect(
            "the floor constraint is gone from the migration. The constructor still refuses 0.05, \
             which means the platform now refuses it in the one place a human types it and \
             accepts it everywhere else.",
        )
        .1;
    let constraint = &constraint[..constraint.find(");").unwrap_or(constraint.len())];

    assert!(
        constraint.contains(&floor),
        "the column's floor and `EXPLORATION_FLOOR` ({floor}) are different numbers. The \
         constraint reads: {constraint}"
    );
    assert!(
        constraint.contains(">="),
        "the constraint no longer states a floor: {constraint}"
    );
}

/// A CHECK that can evaluate to NULL is a CHECK that passes.
///
/// `(charter->>'exploration_share')::double precision` is NULL when the key is
/// absent, and `NULL >= 0.10` is NULL, which Postgres treats as satisfied. A
/// charter written without the key would therefore slip through the one
/// constraint this table exists to hold — so the constraint coalesces to a value
/// below the floor first, and this asserts it still does.
#[test]
fn a_missing_key_is_refused_rather_than_silently_admitted() {
    let constraint = MIGRATION
        .split_once("CONSTRAINT exploration_never_below_the_floor CHECK (")
        .expect("the floor constraint is still there")
        .1;
    let constraint = &constraint[..constraint.find(");").unwrap_or(constraint.len())];
    assert!(
        constraint.to_uppercase().contains("COALESCE"),
        "the constraint reads the key straight out of the document, so a charter missing it \
         evaluates to NULL and is admitted: {constraint}"
    );
}

#[test]
fn the_constructor_refuses_what_the_column_refuses() {
    // Just below.
    assert!(ExplorationShare::new(EXPLORATION_FLOOR - 1e-9).is_err());
    // At.
    assert!(ExplorationShare::new(EXPLORATION_FLOOR).is_ok());
    // The value the acceptance names.
    assert!(ExplorationShare::new(0.05).is_err());
}

/// The concurrency bound is the same shape of rule and the same shape of risk:
/// three children at once is a policy about how fast a day budget can be spent.
#[test]
fn the_concurrency_bound_is_in_both_places() {
    assert!(
        MIGRATION.contains("CONSTRAINT concurrency_is_one_to_three"),
        "the column no longer bounds concurrency"
    );
    assert!(research_session::Schedule::default()
        .with_concurrency(research_session::charter::MAX_CONCURRENT_RUNS)
        .is_ok());
    assert!(research_session::Schedule::default()
        .with_concurrency(research_session::charter::MAX_CONCURRENT_RUNS + 1)
        .is_err());
    let bound = research_session::charter::MAX_CONCURRENT_RUNS.to_string();
    assert!(
        MIGRATION.contains(&format!("BETWEEN 1 AND {bound}")),
        "the column's concurrency bound is not MAX_CONCURRENT_RUNS ({bound})"
    );
}

/// The shape rule — a session is a question or a charter, never both and never
/// neither — is enforced by `ResearchSession::validate` and by a CHECK.
#[test]
fn the_shape_rule_is_in_both_places() {
    assert!(
        MIGRATION.contains("CONSTRAINT a_session_is_a_question_or_a_charter"),
        "the table no longer holds the shape rule, so a continuous session with no charter can be \
         written and nothing that reads it will know what it was supposed to do"
    );
    let mut s = research_session::ResearchSession::bounded(
        uuid::Uuid::nil(),
        uuid::Uuid::nil(),
        "q",
        "opus",
    );
    assert!(s.validate().is_ok());
    s.continuous = true;
    assert!(s.validate().is_err());
}

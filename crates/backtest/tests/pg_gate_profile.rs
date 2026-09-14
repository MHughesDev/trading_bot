//! AT-29: gate profiles are immutable and versioned (SPEC §12.3, INV-23).
//!
//! Ignored by default (needs live Postgres):
//! `cargo test -p backtest --test pg_gate_profile -- --ignored --test-threads=1`
//!
//! The Rust half of this is in `gates::profile` — a type with no setter. The
//! half that matters more is here: the *database* must refuse the edit, because
//! a type only constrains code that goes through it and the threshold table is
//! reachable by anything holding a connection.

use backtest::gates::profile::{Comparability, GateProfile, STRICT_V1};
use sqlx::{Connection, Executor, PgConnection, PgPool, Row};

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");

async fn scratch(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    pool.execute(M0043).await.expect("migration applies");
    pool
}

/// AT-29, first half: an UPDATE to a threshold is refused, and so is a DELETE.
/// This is the mechanism that stops threshold drift from rewriting history.
#[tokio::test]
#[ignore = "requires live postgres"]
async fn a_gate_threshold_cannot_be_edited_or_deleted() {
    let pg = scratch("gate_profile_at29").await;

    let before: serde_json::Value =
        sqlx::query_scalar("SELECT thresholds FROM mlops.gate_profile WHERE profile_id = $1")
            .bind(STRICT_V1)
            .fetch_one(&pg)
            .await
            .expect("strict_v1 was inserted by the migration");

    let update = sqlx::query(
        "UPDATE mlops.gate_profile SET thresholds = jsonb_set(thresholds, '{dsr_gte}', '0.5') \
         WHERE profile_id = $1",
    )
    .bind(STRICT_V1)
    .execute(&pg)
    .await;
    assert!(update.is_err(), "editing a gate threshold must be refused");

    let delete = sqlx::query("DELETE FROM mlops.gate_profile WHERE profile_id = $1")
        .bind(STRICT_V1)
        .execute(&pg)
        .await;
    assert!(delete.is_err(), "deleting a gate profile must be refused");

    let after: serde_json::Value =
        sqlx::query_scalar("SELECT thresholds FROM mlops.gate_profile WHERE profile_id = $1")
            .bind(STRICT_V1)
            .fetch_one(&pg)
            .await
            .unwrap();
    assert_eq!(before, after, "the refused writes changed nothing");
}

/// AT-29, second half: a new version *is* allowed, names what it supersedes, and
/// leaves the old one intact. Changing a threshold has exactly one legal shape.
#[tokio::test]
#[ignore = "requires live postgres"]
async fn creating_a_superseding_profile_succeeds_and_leaves_the_old_one_intact() {
    let pg = scratch("gate_profile_v2").await;

    let v1_row: (serde_json::Value,) =
        sqlx::query_as("SELECT thresholds FROM mlops.gate_profile WHERE profile_id = $1")
            .bind(STRICT_V1)
            .fetch_one(&pg)
            .await
            .unwrap();
    let v1 = GateProfile::from_row(STRICT_V1, None, &v1_row.0)
        .expect("the shipped row parses into a complete profile");

    let mut tightened = *v1.thresholds();
    tightened.dsr_gte = 0.99;
    let v2 = GateProfile::superseding("strict_v2", &v1, tightened);

    sqlx::query(
        "INSERT INTO mlops.gate_profile (profile_id, thresholds, supersedes, created_by) \
         VALUES ($1, $2, $3, 'test')",
    )
    .bind(v2.profile_id())
    .bind(serde_json::to_value(v2.thresholds()).unwrap())
    .bind(v2.supersedes())
    .execute(&pg)
    .await
    .expect("creating a new version is the sanctioned way to change a threshold");

    let rows = sqlx::query("SELECT profile_id, supersedes, thresholds FROM mlops.gate_profile ORDER BY profile_id")
        .fetch_all(&pg)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "both versions exist; nothing was replaced");

    let stored_v1 = rows
        .iter()
        .find(|r| r.get::<String, _>("profile_id") == STRICT_V1)
        .unwrap();
    assert_eq!(
        stored_v1.get::<serde_json::Value, _>("thresholds"),
        v1_row.0,
        "the superseded profile is byte-identical: last year's verdicts still mean what they meant"
    );

    let stored_v2 = rows
        .iter()
        .find(|r| r.get::<String, _>("profile_id") == "strict_v2")
        .unwrap();
    assert_eq!(
        stored_v2.get::<Option<String>, _>("supersedes").as_deref(),
        Some(STRICT_V1)
    );

    // AT-29, third half: a comparison spanning the two is non-comparable.
    let spanning = Comparability::of(STRICT_V1, "strict_v2");
    assert!(!spanning.is_comparable());
    assert!(spanning.notice().is_some());
}

/// A profile with an incomplete threshold set cannot be loaded, so a gate stack
/// can never run against a bar nobody stated.
#[tokio::test]
#[ignore = "requires live postgres"]
async fn an_incomplete_profile_row_cannot_be_loaded() {
    let pg = scratch("gate_profile_partial").await;
    sqlx::query(
        "INSERT INTO mlops.gate_profile (profile_id, thresholds, created_by) \
         VALUES ('partial', '{\"dsr_gte\": 0.95}'::jsonb, 'test')",
    )
    .execute(&pg)
    .await
    .unwrap();

    let row: (serde_json::Value,) =
        sqlx::query_as("SELECT thresholds FROM mlops.gate_profile WHERE profile_id = 'partial'")
            .fetch_one(&pg)
            .await
            .unwrap();
    assert!(
        GateProfile::from_row("partial", None, &row.0).is_err(),
        "a profile missing thresholds must fail to load, not default them"
    );
}

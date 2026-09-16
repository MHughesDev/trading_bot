//! `MemoryStore` writes rows the schema accepts and reads back what it wrote.
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test memory_store_round_trip -- --ignored --test-threads=1
//! ```
//!
//! # Why this needs a database and the unit tests do not
//!
//! `memory_store`'s unit tests cover the enum round trips — every `Kind`,
//! `Confidence`, `EvidenceClass` and `Direction` survives its string — with no
//! database, because that is pure mapping.
//!
//! What they cannot cover is the part most likely to be wrong: `put` binds
//! **twenty-nine columns positionally**. A name misspelled, a pair transposed,
//! a type that does not match its column — none of that is visible to the
//! compiler, and all of it fails at the first real write. An integration test is
//! the only thing that reads that INSERT the way Postgres does.
//!
//! The round trip is the assertion rather than the insert, because a write that
//! lands in the wrong column still succeeds. Only reading it back catches
//! `subject` and `claim` swapped.

use harness::memory::{
    Confidence, DataWindow, Direction, Entry, EvidenceClass, Kind, Measurement, Provenance,
};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// See `crates/invariants/tests/semantic_memory_refuses.rs` for why this applies
/// one migration rather than the chain: `0055_knowledge_plane.sql` needs
/// pgvector, which a stock Postgres does not have, so the chain cannot reach
/// 0122 on a developer machine.
async fn store_db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS memory_store_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE memory_store_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "memory_store_test"))
        .await
        .unwrap();

    let has_vector = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0)
        > 0;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0122_semantic_memory.sql");
    let mut sql = std::fs::read_to_string(&path).expect("the migration is readable");
    if has_vector {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&pool)
            .await
            .expect("pgvector installs");
    } else {
        println!("NOTE: pgvector absent; `embedding` applied as bytea. No column under test.");
        sql = sql.replace("embedding       vector(768),", "embedding       bytea,");
    }
    pool.execute(sql.as_str())
        .await
        .expect("the migration applies");
    pool
}

fn full_entry(id: &str) -> Entry {
    let mut e = Entry::finding(
        id,
        "BTC-USD 1h",
        "the residual t-stat is 3.1 after Newey-West",
        DataWindow::new("2026-04-01", "2026-06-30"),
        Provenance::new("run_9", "run_backtest", "3", "hash_abc"),
    );
    e.measurement = Some(Measurement {
        quantity: "residual_t".into(),
        direction: Direction::Positive,
        value: 3.1,
        se: Some(0.4),
        n: Some(812),
    });
    e.confidence = Confidence::Suggestive;
    e.evidence_class = EvidenceClass::Derived;
    e.evidence_ids
        .insert("experiment_id".into(), "exp_42".into());
    e.valid_until = Some("2026-09-28".into());
    e.created_by = "agent:opus".into();
    e.conflicts_with = vec!["f_7".into(), "f_9".into()];
    e.corroborated_by = vec!["run_10".into()];
    e.corroborations = 2;
    e.stale = true;
    e.cited_by_chain = true;
    e.extra = serde_json::json!({ "failure_class": "weak_effect" });
    e
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn every_field_survives_the_round_trip() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let user = Uuid::new_v4();

    let written = full_entry("f_round");
    store.put(user, &written).await.expect("the entry stores");

    let back = store
        .by_kind(user, Kind::Finding, 10)
        .await
        .expect("the entry reads back");
    assert_eq!(back.len(), 1, "one written, one read");
    let r = &back[0];

    // Field by field rather than a whole-struct compare, so a failure names the
    // column. A positional INSERT that transposed two of twenty-nine would
    // otherwise report "entries differ" and leave the reader to diff them.
    assert_eq!(r.id, written.id, "entry_id");
    assert_eq!(r.kind, written.kind, "kind");
    assert_eq!(r.subject, written.subject, "subject");
    assert_eq!(r.claim, written.claim, "claim — transposed with subject?");
    assert_eq!(r.confidence, written.confidence, "confidence");
    assert_eq!(r.evidence_class, written.evidence_class, "evidence_class");
    assert_eq!(r.evidence_ids, written.evidence_ids, "evidence_ids");
    assert_eq!(r.data_window, written.data_window, "data_window");
    assert_eq!(r.valid_until, written.valid_until, "valid_until");
    assert_eq!(r.provenance, written.provenance, "provenance — all four parts");
    assert_eq!(r.created_by, written.created_by, "created_by");
    assert_eq!(r.conflicts_with, written.conflicts_with, "conflicts_with");
    assert_eq!(r.corroborated_by, written.corroborated_by, "corroborated_by");
    assert_eq!(r.corroborations, written.corroborations, "corroborations");
    assert_eq!(r.stale, written.stale, "stale");
    assert_eq!(r.cited_by_chain, written.cited_by_chain, "cited_by_chain");
    assert_eq!(r.extra, written.extra, "extra");

    let m = r.measurement.as_ref().expect("the measurement survives");
    let w = written.measurement.as_ref().unwrap();
    assert_eq!(m.quantity, w.quantity, "quantity");
    assert_eq!(m.direction, w.direction, "direction");
    assert!((m.value - w.value).abs() < 1e-12, "value");
    assert_eq!(m.se, w.se, "se");
    assert_eq!(m.n, w.n, "n");
}

/// An entry `Entry::validate` refuses never reaches the database.
///
/// The CHECK would refuse it too — that is asserted in
/// `semantic_memory_refuses.rs`. What this proves is that the caller gets
/// `EntryError::ClaimTooLong(301)` rather than a `sqlx::Error` naming
/// `entry_claim_check`, which is the difference between a message someone can
/// act on and a constraint name they have to go and look up.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_invalid_entry_is_refused_before_the_database_sees_it() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);

    let mut bad = full_entry("f_bad");
    bad.claim = "x".repeat(301);
    let err = store
        .put(Uuid::new_v4(), &bad)
        .await
        .expect_err("a 301-character claim is refused");
    let text = err.to_string();
    assert!(
        text.contains("301") && text.contains("300"),
        "the caller should be told the length and the limit, not handed a constraint name: \
         {text}"
    );
}

/// The store scopes to one user.
///
/// Memory is per-user and there is no cross-tenant read path. A missing
/// `user_id` in the WHERE clause is the kind of omission that works perfectly in
/// every single-user test.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_users_memory_is_not_another_users() {
    let pool = store_db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());

    store.put(a, &full_entry("f_a")).await.expect("stores");
    let seen = store
        .by_kind(b, Kind::Finding, 10)
        .await
        .expect("the query runs");
    assert!(
        seen.is_empty(),
        "user b read {} of user a's entries",
        seen.len()
    );

    let mine = store.for_subject(a, "BTC-USD 1h", 10).await.expect("runs");
    assert_eq!(mine.len(), 1, "and user a still sees their own");
    assert_eq!(mine[0].claim, full_entry("f_a").claim, "and it came back whole");
}

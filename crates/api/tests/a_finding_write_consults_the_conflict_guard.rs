//! The memory write path consults the conflict guard (S-4.15, S-4.16).
//!
//! `conflict::decide_write` was complete, unit-tested, and called by nothing:
//! `conflicts_with` was a column no code would ever populate. These tests drive
//! the real `MemoryStore::put` (the door `Effect::Remember` goes through) against
//! Postgres and read the rows back.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres //!   cargo test -j 2 -p api --test a_finding_write_consults_the_conflict_guard -- --ignored --test-threads=1
//! ```

use api::memory_store::{MemoryStore, WriteOutcome};
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
        .execute("DROP DATABASE IF EXISTS memory_conflict_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE memory_conflict_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "memory_conflict_test"))
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

    // The negative-result constraints ride on the same table (S-4.4).
    let class_sql = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crates/<name> sits two below the root")
            .join("migrations/0125_negative_result_class.sql"),
    )
    .expect("0125 is readable");
    pool.execute(class_sql.as_str())
        .await
        .expect("0125_negative_result_class.sql applies");
    let retrievals = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crates/<name> sits two below the root")
            .join("migrations/0124_memory_retrievals.sql"),
    )
    .expect("0124 is readable");
    pool.execute(retrievals.as_str())
        .await
        .expect("0124_memory_retrievals.sql applies");
    pool
}


fn finding(id: &str, run: &str, dir: Direction, value: f64, se: f64, window: (&str, &str)) -> Entry {
    let mut e = Entry::finding(
        id,
        "BTC-USD 1h",
        "momentum residual",
        DataWindow::new(window.0, window.1),
        Provenance::new(run, "run_backtest", "3", "hash_abc"),
    );
    e.measurement = Some(Measurement {
        quantity: "residual_t".into(),
        direction: dir,
        value,
        se: Some(se),
        n: Some(500),
    });
    e.confidence = Confidence::Established;
    e.evidence_class = EvidenceClass::DirectStat;
    e.created_by = "agent:test".into();
    e
}

const W: (&str, &str) = ("2026-04-01", "2026-06-30");

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_opposing_finding_links_both_rows_and_downgrades_neither() {
    let store = MemoryStore::new(store_db().await);
    let user = Uuid::new_v4();
    store
        .put(user, &finding("f_a", "run_1", Direction::Positive, 3.0, 0.3, W))
        .await
        .unwrap();
    let out = store
        .put_checked(user, &finding("f_b", "run_2", Direction::Negative, -3.0, 0.3, W))
        .await
        .unwrap();
    assert_eq!(out, WriteOutcome::Inserted { conflicts_with: vec!["f_a".into()] });

    let a = store.get(user, "f_a").await.unwrap().unwrap();
    let b = store.get(user, "f_b").await.unwrap().unwrap();
    assert_eq!(a.conflicts_with, vec!["f_b".to_string()], "the stored row is linked back");
    assert_eq!(b.conflicts_with, vec!["f_a".to_string()]);
    assert_eq!(a.confidence, Confidence::Established, "neither is downgraded");
    assert_eq!(b.confidence, Confidence::Established);
    // Both enter the brief's view together.
    let live = store.for_subject(user, "BTC-USD 1h", 10).await.unwrap();
    assert_eq!(live.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn overlapping_interval_disagreement_is_not_flagged() {
    let store = MemoryStore::new(store_db().await);
    let user = Uuid::new_v4();
    store
        .put(user, &finding("f_a", "run_1", Direction::Positive, 0.4, 0.5, W))
        .await
        .unwrap();
    let out = store
        .put_checked(user, &finding("f_b", "run_2", Direction::Negative, -0.4, 0.5, W))
        .await
        .unwrap();
    assert_eq!(out, WriteOutcome::Inserted { conflicts_with: vec![] });
    assert!(store.get(user, "f_a").await.unwrap().unwrap().conflicts_with.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_near_duplicate_increments_corroboration_rather_than_inserting() {
    let store = MemoryStore::new(store_db().await);
    let user = Uuid::new_v4();
    store
        .put(user, &finding("f_a", "run_1", Direction::Positive, 3.0, 0.3, W))
        .await
        .unwrap();
    let out = store
        .put_checked(user, &finding("f_dup", "run_2", Direction::Positive, 3.1, 0.3, W))
        .await
        .unwrap();
    assert_eq!(out, WriteOutcome::Corroborated { into: "f_a".into() });

    assert!(store.get(user, "f_dup").await.unwrap().is_none(), "no row for the duplicate");
    let a = store.get(user, "f_a").await.unwrap().unwrap();
    assert_eq!(a.corroborations, 2);
    assert!(a.corroborated_by.contains(&"run_2".to_string()));
    assert_eq!(store.for_subject(user, "BTC-USD 1h", 10).await.unwrap().len(), 1);
    let _ = Kind::Finding;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn rewriting_the_same_id_is_an_upsert_not_a_corroboration() {
    let store = MemoryStore::new(store_db().await);
    let user = Uuid::new_v4();
    let e = finding("f_a", "run_1", Direction::Positive, 3.0, 0.3, W);
    store.put(user, &e).await.unwrap();
    let out = store.put_checked(user, &e).await.unwrap();
    assert!(matches!(out, WriteOutcome::Inserted { .. }));
    assert_eq!(store.get(user, "f_a").await.unwrap().unwrap().corroborations, 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_retention_sweep_archives_a_stale_uncited_finding_but_the_id_still_resolves() {
    let store = MemoryStore::new(store_db().await);
    let user = Uuid::new_v4();
    let mut old = finding("f_old", "run_1", Direction::Positive, 3.0, 0.3, W);
    old.valid_until = Some("2026-07-01".into());
    store.put(user, &old).await.unwrap();
    let mut fresh = finding("f_fresh", "run_1", Direction::Positive, 1.0, 0.3, W);
    fresh.subject = "ETH-USD 1h".into();
    store.put(user, &fresh).await.unwrap();
    let mut neg = Entry::finding(
        "n_1",
        "SOL-USD 1h",
        "no edge",
        DataWindow::new(W.0, W.1),
        Provenance::new("run_1", "run_backtest", "3", "hash_abc"),
    );
    neg.kind = Kind::NegativeResult;
    neg.valid_until = Some("2026-07-01".into());
    neg.extra = serde_json::json!({"failure_class": "weak_effect"});
    let _ = store.put(user, &neg).await;

    // Far enough on that the 180-day uncited window has passed.
    let r = store.sweep_retention("2027-06-01").await.unwrap();
    assert!(r.archived_now >= 1, "{r:?}");

    let live = store.for_subject(user, "BTC-USD 1h", 10).await.unwrap();
    assert!(live.is_empty(), "an archived finding leaves the brief");
    let still = store.get(user, "f_old").await.unwrap().expect("still resolves by id");
    assert!(still.archived);
    // Its sibling with no expiry stays live.
    assert_eq!(store.for_subject(user, "ETH-USD 1h", 10).await.unwrap().len(), 1);

    let rep = store.store_report(user).await.unwrap();
    assert!(rep.total >= 2 && rep.archived >= 1);
    assert!(rep.archive_rate > 0.0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_always_start_preference_skips_the_offer_until_it_is_deleted() {
    let pool = store_db().await;
    let store = MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();
    assert!(!store.prefers_always_start(user, "BTC-USD").await.unwrap());

    let mut p = Entry::finding(
        "pref_start",
        "BTC-USD",
        "always start research on BTC-USD without asking",
        DataWindow::new("2026-10-01", "2026-10-01"),
        Provenance::new("user", "ask_user", "1", "n/a"),
    );
    p.kind = Kind::UserPreference;
    p.confidence = Confidence::Established;
    p.evidence_class = EvidenceClass::Human;
    p.created_by = user.to_string();
    p.extra = serde_json::json!({"directive": api::memory_store::ALWAYS_START_DIRECTIVE});
    store.put(user, &p).await.unwrap();

    assert!(store.prefers_always_start(user, "BTC-USD").await.unwrap());
    assert!(!store.prefers_always_start(user, "ETH-USD").await.unwrap(), "scoped");
    assert!(!store.prefers_always_start(Uuid::new_v4(), "BTC-USD").await.unwrap(), "per user");

    sqlx::query("UPDATE agent_memory.entry SET archived = true WHERE entry_id = 'pref_start'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!store.prefers_always_start(user, "BTC-USD").await.unwrap(), "deleted restores the offer");
}

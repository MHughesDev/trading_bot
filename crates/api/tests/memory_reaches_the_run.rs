//! A stored preference reaches the run that needs it (AGENT-007 §4; S-4.5, S-4.6).
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test memory_reaches_the_run -- --ignored --test-threads=1
//! ```
//!
//! # Why this test and not a unit test of `assemble_brief`
//!
//! `memory/brief.rs` has fourteen tests and they all pass. Until now it also had
//! **no caller anywhere in the workspace** — 936 lines of tested, exported,
//! never-executed code. A module can be entirely correct and entirely unreached,
//! and its own tests cannot tell you which.
//!
//! So this walks the whole chain rather than any link in it: a preference is
//! written through `MemoryStore`, read back the way the driver reads it,
//! assembled the way the driver assembles it, and the result is checked for the
//! user's own words. Every step is the production path; only the driver's outer
//! shell (an LLM, a registry, a workspace) is absent.

use harness::memory::{Confidence, DataWindow, Entry, EvidenceClass, Kind, Provenance};
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

/// Both tables this chain touches. The full chain needs pgvector, which a stock
/// Postgres lacks — see `crates/invariants/tests/semantic_memory_refuses.rs`.
async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS memory_chain_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE memory_chain_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "memory_chain_test"))
        .await
        .unwrap();

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root");
    let has_vector = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_available_extensions WHERE name = 'vector'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0)
        > 0;
    if has_vector {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&pool)
            .await
            .expect("pgvector installs");
    }

    for file in [
        "migrations/0122_semantic_memory.sql",
        "migrations/0124_memory_retrievals.sql",
    ] {
        let mut sql = std::fs::read_to_string(root.join(file)).expect("the migration is readable");
        if !has_vector {
            sql = sql.replace("embedding       vector(768),", "embedding       bytea,");
        }
        pool.execute(sql.as_str())
            .await
            .unwrap_or_else(|e| panic!("{file} does not apply: {e}"));
    }
    pool
}

fn preference(id: &str, scope: &str, statement: &str, user: Uuid) -> Entry {
    let today = chrono::Utc::now().date_naive().to_string();
    let mut e = Entry::finding(
        id,
        scope,
        statement,
        DataWindow::new(today.clone(), today),
        Provenance::new("user", "ask_user", "1", "n/a"),
    );
    e.kind = Kind::UserPreference;
    e.confidence = Confidence::Established;
    e.evidence_class = EvidenceClass::Human;
    e.created_by = user.to_string();
    e
}

/// The chain, end to end.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_stored_preference_reaches_the_brief_and_is_logged_as_retrieved() {
    let pool = db().await;
    let store = api::memory_store::MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();

    store
        .put(
            user,
            &preference(
                "pref_weekend",
                "BTC-USD",
                "never hold BTC-USD through a weekend gap",
                user,
            ),
        )
        .await
        .expect("the preference stores");

    // Read the way the driver reads: by kind, live rows only.
    let entries = store
        .by_kind(user, Kind::UserPreference, 200)
        .await
        .expect("the driver's read");
    assert_eq!(entries.len(), 1, "one preference to draw on");

    // Assembled the way the driver assembles.
    let run_id = Uuid::new_v4().to_string();
    let brief = harness::memory::assemble_brief(
        &run_id,
        "find an edge in BTC-USD over the weekend",
        &entries,
        4_000,
        harness::memory::BriefDetail::Lines,
        &chrono::Utc::now().date_naive().to_string(),
        &[],
        &[],
    );

    assert!(
        !brief.retrievals.is_empty(),
        "the brief retrieved nothing from a store holding a preference about the very \
         instrument in the goal"
    );

    // The user's own words reach the prompt.
    let rendered: String = brief
        .blocks()
        .iter()
        .map(|b| format!("{:?}", b.tagged))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("weekend gap"),
        "the preference did not reach the rendered blocks:\n{rendered}"
    );

    // And the run is on record as having been shown it, which is the
    // denominator `memory_recall` divides by.
    let retrievals = api::retrieval_store::RetrievalStore::new(pool);
    retrievals.log(&brief.retrievals).await.expect("logs");
    assert_eq!(
        retrievals.recall(&run_id).await.expect("the share reads"),
        Some(0.0),
        "shown and not yet cited is a recall of zero, not an absence of one"
    );

    let moved = retrievals
        .mark_cited(&run_id, &["pref_weekend".into()])
        .await
        .expect("the citation lands");
    assert_eq!(moved, 1, "the entry the brief showed is the entry cited");
    assert_eq!(retrievals.recall(&run_id).await.unwrap(), Some(1.0));
}

/// A deleted preference does not reach the next brief.
///
/// S-4.6's acceptance, asserted at the end of the chain rather than on the
/// `archived` flag: a test of the flag passes while the brief still shows it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_deleted_preference_does_not_reach_the_next_brief() {
    let pool = db().await;
    let store = api::memory_store::MemoryStore::new(pool.clone());
    let user = Uuid::new_v4();

    store
        .put(user, &preference("pref_x", "BTC-USD", "avoid weekend gaps", user))
        .await
        .expect("stores");
    sqlx::query("UPDATE agent_memory.entry SET archived = true WHERE entry_id = 'pref_x'")
        .execute(&pool)
        .await
        .expect("the delete lands");

    let entries = store
        .by_kind(user, Kind::UserPreference, 200)
        .await
        .expect("the driver's read");
    assert!(entries.is_empty(), "the deleted preference is still readable");

    let brief = harness::memory::assemble_brief(
        &Uuid::new_v4().to_string(),
        "find an edge in BTC-USD over the weekend",
        &entries,
        4_000,
        harness::memory::BriefDetail::Lines,
        &chrono::Utc::now().date_naive().to_string(),
        &[],
        &[],
    );
    let rendered: String = brief
        .blocks()
        .iter()
        .map(|b| format!("{:?}", b.tagged))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rendered.contains("weekend"),
        "a deleted preference reached the brief anyway:\n{rendered}"
    );
}

/// One user's memory never reaches another user's run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_users_memory_never_reaches_anothers_run() {
    let pool = db().await;
    let store = api::memory_store::MemoryStore::new(pool);
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    store
        .put(a, &preference("pref_a", "BTC-USD", "a private habit", a))
        .await
        .expect("stores");

    let for_b = store
        .by_kind(b, Kind::UserPreference, 200)
        .await
        .expect("reads");
    assert!(
        for_b.is_empty(),
        "user b's brief would have been built from user a's preferences"
    );
}

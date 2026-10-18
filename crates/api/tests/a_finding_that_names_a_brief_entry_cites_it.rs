//! A finding whose evidence names an entry the brief showed marks it cited
//! (AGENT-007 §4; Set S task S-6.10).
//!
//! `RetrievalStore::mark_cited` existed, tested, with no production caller, so
//! the desk's Memory tab could only say "citation not recorded". The caller is
//! `local_driver::record_citations`, which the run recorder invokes for every
//! `record_finding` / `finish_task` note. This walks that function against a
//! real table: the retrieval log is written the way `load_brief` writes it, the
//! citation is recorded the way the recorder records it, and the row is read
//! back the way the desk's route reads it.
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test a_finding_that_names_a_brief_entry_cites_it -- --ignored
//! ```
//! `DATABASE_URL` selects the server (default: the native PG 17 on 5433); the
//! scratch database is dropped at the end.

use api::retrieval_store::record_citations;
use api::retrieval_store::RetrievalStore;
use harness::memory::{Retrieval, Slot};
use serde_json::json;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/postgres".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const DB: &str = "citation_recorded_test";

fn shown(run: &str, id: &str, rank: usize) -> Retrieval {
    Retrieval {
        run_id: run.to_string(),
        entry_id: id.to_string(),
        rank,
        slot: Slot::Finding,
        why: "matched the goal".to_string(),
        cited: false,
    }
}

#[tokio::test]
#[ignore = "needs Postgres"]
async fn a_finding_that_names_a_brief_entry_cites_it() {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {DB}").as_str())
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), DB)).await.unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root");
    let sql = std::fs::read_to_string(root.join("migrations/0124_memory_retrievals.sql")).unwrap();
    pool.execute(sql.as_str()).await.unwrap();

    let run_id = Uuid::new_v4();
    let run = run_id.to_string();
    let retrievals = vec![
        shown(&run, "f_1", 1),
        shown(&run, "f_2", 2),
        shown(&run, "q_7", 1),
    ];
    let store = RetrievalStore::new(pool.clone());
    store.log(&retrievals).await.unwrap();
    assert_eq!(
        store.recall(&run).await.unwrap(),
        Some(0.0),
        "nothing is cited yet"
    );

    // A tool that carries no evidence is not a citation, whatever it names.
    record_citations(
        &pool,
        run_id,
        &retrievals,
        "search_tools",
        &json!({"evidence": "f_1"}),
    )
    .await;
    assert_eq!(store.recall(&run).await.unwrap(), Some(0.0));

    // A finding naming one shown id, one that was never shown, and a longer id
    // that merely starts with a shown one.
    record_citations(
        &pool,
        run_id,
        &retrievals,
        "record_finding",
        &json!({"finding": "x", "evidence": "f_1, f_99, f_22"}),
    )
    .await;
    let cited: Vec<String> = sqlx::query_scalar(
        "SELECT entry_id FROM memory_retrievals WHERE run_id = $1 AND cited ORDER BY entry_id",
    )
    .bind(&run)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(cited, vec!["f_1"], "only the whole id the brief showed");

    // finish_task cites too, and a second mention does not move cited_at.
    let first: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT cited_at FROM memory_retrievals WHERE run_id = $1 AND entry_id = 'f_1'",
    )
    .bind(&run)
    .fetch_one(&pool)
    .await
    .unwrap();
    record_citations(
        &pool,
        run_id,
        &retrievals,
        "finish_task",
        &json!({"result": "done", "evidence": "q_7 and f_1"}),
    )
    .await;
    assert_eq!(store.recall(&run).await.unwrap(), Some(2.0 / 3.0));
    let again: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT cited_at FROM memory_retrievals WHERE run_id = $1 AND entry_id = 'f_1'",
    )
    .bind(&run)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first, again, "the first citation sets the moment");

    pool.close().await;
    admin
        .execute(format!("DROP DATABASE IF EXISTS {DB} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

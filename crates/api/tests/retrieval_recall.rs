//! `memory_retrievals.cited` is set, and the recall share follows from it
//! (AGENT-007 §4; AGENT-009 §4; Set S task S-4.5).
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test retrieval_recall -- --ignored --test-threads=1
//! ```
//!
//! # The acceptance clause this covers
//!
//! S-4.5 asks that "`memory_retrievals.cited` is set when a later finding or
//! `finish_task` names the entry". That is two writes to one row by two
//! different parts of a run, and the second is the measurement — so it cannot be
//! checked by looking at either write alone.

use harness::memory::brief::{Retrieval, Slot};
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// Applies the one migration under test — the chain needs pgvector, which a
/// stock Postgres lacks. See `crates/invariants/tests/semantic_memory_refuses.rs`.
async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS retrieval_recall_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE retrieval_recall_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "retrieval_recall_test"))
        .await
        .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0124_memory_retrievals.sql");
    let sql = std::fs::read_to_string(&path).expect("the migration is readable");
    pool.execute(sql.as_str())
        .await
        .expect("0124_memory_retrievals.sql applies to an empty database");
    pool
}

fn shown(run: &str, entry: &str, rank: usize, slot: Slot) -> Retrieval {
    Retrieval {
        run_id: run.into(),
        entry_id: entry.into(),
        rank,
        slot,
        why: format!("matched the goal on `{entry}`"),
        cited: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn citing_an_entry_sets_cited_and_moves_the_share() {
    let store = api::retrieval_store::RetrievalStore::new(db().await);

    store
        .log(&[
            shown("run_1", "f_1", 1, Slot::Finding),
            shown("run_1", "f_2", 2, Slot::Finding),
            shown("run_1", "n_1", 1, Slot::NegativeResult),
            shown("run_1", "p_1", 1, Slot::UserPreference),
        ])
        .await
        .expect("the brief logs what it showed");

    // Nothing cited yet: four shown, none used.
    assert_eq!(
        store.recall("run_1").await.expect("the share reads"),
        Some(0.0),
        "four entries were shown and none cited, which is a recall of zero — not an \
         absence of one"
    );

    let moved = store
        .mark_cited("run_1", &["f_1".into(), "n_1".into()])
        .await
        .expect("the citation lands");
    assert_eq!(moved, 2, "two rows moved");
    assert_eq!(store.recall("run_1").await.unwrap(), Some(0.5));

    // Citing the same entry again is not a second hit.
    let again = store
        .mark_cited("run_1", &["f_1".into()])
        .await
        .expect("the second citation runs");
    assert_eq!(again, 0, "an entry already cited is not cited twice");
    assert_eq!(store.recall("run_1").await.unwrap(), Some(0.5));
}

/// A run that retrieved nothing has not failed at recall.
///
/// `None` and `Some(0.0)` are different answers, and folding them together would
/// make the score improve whenever the store was empty — the direction that
/// hides the problem.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_run_that_retrieved_nothing_scores_none_rather_than_zero() {
    let store = api::retrieval_store::RetrievalStore::new(db().await);
    assert_eq!(
        store.recall("run_empty").await.expect("the query runs"),
        None
    );
}

/// Citing an entry the brief never showed moves no rows, and the count says so.
///
/// This is a different failure from a low share and looks identical in the
/// score: the model named an id it did not get from memory. Returning the count
/// rather than discarding it is what makes the two distinguishable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn citing_something_never_shown_moves_nothing() {
    let store = api::retrieval_store::RetrievalStore::new(db().await);
    store
        .log(&[shown("run_2", "f_1", 1, Slot::Finding)])
        .await
        .expect("logs");
    let moved = store
        .mark_cited("run_2", &["f_99".into()])
        .await
        .expect("the update runs");
    assert_eq!(
        moved, 0,
        "`f_99` was never retrieved in this run, so nothing should be marked"
    );
    assert_eq!(store.recall("run_2").await.unwrap(), Some(0.0));
}

/// A brief assembled twice in one run counts each entry once.
///
/// A re-plan or a resumed step reassembles the brief. Counting the same entry
/// twice would make a run that re-planned score differently from one that did
/// not, for reasons having nothing to do with memory.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn reassembling_the_brief_does_not_double_count() {
    let store = api::retrieval_store::RetrievalStore::new(db().await);
    store
        .log(&[shown("run_3", "f_1", 1, Slot::Finding)])
        .await
        .expect("logs");
    // The same entry, a different rank the second time.
    store
        .log(&[shown("run_3", "f_1", 7, Slot::Finding)])
        .await
        .expect("logs again");

    store
        .mark_cited("run_3", &["f_1".into()])
        .await
        .expect("cites");
    assert_eq!(
        store.recall("run_3").await.unwrap(),
        Some(1.0),
        "one entry, shown twice, cited once — a recall of one, not a half"
    );
}

/// The diagnostic the score cannot give: what the brief keeps showing and the
/// model keeps ignoring.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_entry_shown_repeatedly_and_never_cited_is_findable() {
    let store = api::retrieval_store::RetrievalStore::new(db().await);
    for run in 1..=5 {
        store
            .log(&[
                shown(&format!("r{run}"), "f_useless", 1, Slot::Finding),
                shown(&format!("r{run}"), "f_used", 2, Slot::Finding),
            ])
            .await
            .expect("logs");
        store
            .mark_cited(&format!("r{run}"), &["f_used".into()])
            .await
            .expect("cites");
    }

    let dead = store.never_cited(3).await.expect("the query runs");
    assert_eq!(dead.len(), 1, "one entry is shown often and never used: {dead:?}");
    assert_eq!(dead[0].0, "f_useless");
    assert_eq!(dead[0].1, 5, "shown in all five runs");
}

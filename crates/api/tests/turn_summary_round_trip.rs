//! A turn's conclusions survive the process boundary (Set S task S-4.1).
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p api --test turn_summary_round_trip -- --ignored --test-threads=1
//! ```
//!
//! # The acceptance, and why it needs two turns
//!
//! S-4.1's acceptance is that `prior_context` becomes a renderer over the table
//! and that **a second turn receives the first's conclusions with ids**. Both
//! halves are about what crosses the boundary: a render test alone proves the
//! text is right about a struct held in memory, which is the easy half and not
//! the one that was broken.
//!
//! What was broken is that the carry came from `agent_runs.summary` — the
//! model's own closing prose — so the next turn was told it had concluded
//! whatever the last turn *said* it concluded. This writes turn one the way the
//! harness would, reads it back the way turn two would, and asserts the ids
//! arrive.

use harness::memory::{Confidence, SummaryFinding, SummaryNegative, TurnBudget, TurnSummary};
use harness::state::Stage;
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

/// Applies the one migration under test. See
/// `crates/invariants/tests/semantic_memory_refuses.rs` for why the whole chain
/// cannot run on a stock Postgres: `0055_knowledge_plane.sql` needs pgvector.
/// Unlike 0122, this migration has no vector column, so nothing is substituted.
async fn store_db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS turn_summary_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE turn_summary_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "turn_summary_test"))
        .await
        .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0123_agent_turn_summaries.sql");
    let sql = std::fs::read_to_string(&path).expect("the migration is readable");
    pool.execute(sql.as_str())
        .await
        .expect("0123_agent_turn_summaries.sql applies to an empty database");
    pool
}

fn turn_one() -> TurnSummary {
    TurnSummary {
        turn: 1,
        goal: "is there an edge in BTC-USD 1h".into(),
        outcome: "finished".into(),
        findings: vec![SummaryFinding {
            id: "f_1".into(),
            claim: "the residual t-stat is 3.1 after Newey-West".into(),
            ids: std::collections::BTreeMap::from([(
                "experiment_id".to_string(),
                "exp_42".to_string(),
            )]),
            n: Some(812),
            se: Some(0.4),
            confidence: Confidence::Established,
        }],
        negative_results: vec![SummaryNegative {
            ticket_id: "t_9".into(),
            class: "weak_effect".into(),
            why: "the effect vanished out of sample".into(),
            retry_condition: Some("a Kraken execution adapter exists".into()),
        }],
        trials_spent: 2,
        n_eff_after: Some(7.5),
        gate_states: std::collections::BTreeMap::from([("gate3".to_string(), "open".to_string())]),
        open_questions: vec!["does it survive a costed fill?".into()],
        rejected_ideas: vec![],
        budget: TurnBudget::default(),
        yield_ref: None,
        stage: Stage::default(),
        current_step: Some("design a costed backtest".into()),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_second_turn_receives_the_firsts_conclusions_with_ids() {
    let pool = store_db().await;
    let store = api::turn_summary_store::TurnSummaryStore::new(pool);
    let conversation = Uuid::new_v4();

    store
        .put(conversation, &turn_one())
        .await
        .expect("turn one's summary stores");

    // Turn two asks for what came before it.
    let prior = store
        .before(conversation, 2)
        .await
        .expect("turn two reads the carry");
    assert_eq!(prior.len(), 1, "one turn happened, one is carried");

    let text = api::turn_summary_store::render(&prior);

    // The acceptance names ids specifically, and this is why: a claim without
    // one is a claim the next turn can only take on trust.
    for expected in [
        "f_1",
        "exp_42",
        "t_9",
        "established",
        "the residual t-stat is 3.1",
        "n=812",
    ] {
        assert!(
            text.contains(expected),
            "the carry is missing `{expected}`:\n{text}"
        );
    }
    assert!(
        text.contains("2 trial(s) spent"),
        "and what it cost:\n{text}"
    );
    assert!(
        text.contains("does it survive a costed fill?"),
        "and what it left open:\n{text}"
    );
    assert!(
        text.contains("left off at: design a costed backtest"),
        "and where it stopped:\n{text}"
    );
}

/// Turn one sees nothing, because nothing came before it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_first_turn_carries_nothing() {
    let pool = store_db().await;
    let store = api::turn_summary_store::TurnSummaryStore::new(pool);
    let conversation = Uuid::new_v4();
    store.put(conversation, &turn_one()).await.expect("stores");

    let prior = store.before(conversation, 1).await.expect("query runs");
    assert!(prior.is_empty(), "turn 1 must not carry turn 1");
    assert!(api::turn_summary_store::render(&prior).is_empty());
}

/// A retried turn replaces its summary rather than appending a second one.
///
/// The carry a later turn reads has to be what the turn *finally* did. An
/// append-only table here would make "the previous turn" a question with two
/// answers, and the renderer would show both.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn retrying_a_turn_leaves_one_summary_not_two() {
    let pool = store_db().await;
    let store = api::turn_summary_store::TurnSummaryStore::new(pool);
    let conversation = Uuid::new_v4();

    store.put(conversation, &turn_one()).await.expect("stores");

    let mut retried = turn_one();
    retried.outcome = "refused".into();
    retried.trials_spent = 0;
    retried.findings.clear();
    store.put(conversation, &retried).await.expect("replaces");

    let prior = store.before(conversation, 2).await.expect("query runs");
    assert_eq!(prior.len(), 1, "one turn, one summary");
    assert_eq!(prior[0].outcome, "refused", "the later attempt wins");
    assert!(
        prior[0].findings.is_empty(),
        "and the earlier attempt's findings do not survive it"
    );
}

/// One conversation's carry is not another's.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_conversations_carry_is_not_anothers() {
    let pool = store_db().await;
    let store = api::turn_summary_store::TurnSummaryStore::new(pool);
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    store.put(a, &turn_one()).await.expect("stores");
    assert!(
        store.before(b, 99).await.expect("query runs").is_empty(),
        "conversation b read conversation a's turns"
    );
}

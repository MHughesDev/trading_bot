//! The attribution corpus is written, read back reproducibly, fitted and promoted
//! (S-9.10, S-9.12; migration 0238), through the platform's own role.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test context_corpus_store -- --ignored --test-threads=1
//! ```

use api::context_corpus::{self, RecordRow, RunEntry};
use harness::context::select::Signals;
use harness::context::store::Utility;
use harness::eval::scorecard::{Scorecard, SuiteKind, SuiteScore};
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

async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS context_corpus_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE context_corpus_test")
        .await
        .unwrap();
    let _ = admin
        .execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='app_role') \
             THEN CREATE ROLE app_role NOLOGIN; END IF; END $$",
        )
        .await;
    let pool = PgPool::connect(&with_db(&base_url(), "context_corpus_test"))
        .await
        .unwrap();
    pool.execute(
        "CREATE TABLE agent_runs (id BIGSERIAL PRIMARY KEY, model TEXT, \
           prompt_tokens BIGINT, est_prompt_tokens BIGINT, \
           created_at TIMESTAMPTZ NOT NULL DEFAULT now())",
    )
    .await
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("two below the root");
    for m in [
        "0177_eval_calibration_and_holdout.sql",
        "0238_context_attribution_corpus.sql",
    ] {
        let sql = std::fs::read_to_string(root.join("migrations").join(m)).expect("readable");
        pool.execute(sql.as_str())
            .await
            .unwrap_or_else(|e| panic!("{m}: {e}"));
    }
    pool
}

/// `n_used` records the model used (high id overlap) and `n_unused` it did not.
fn entry(
    policy: &str,
    profile: &str,
    used_t: u64,
    rendered_t: u64,
    n_used: usize,
    n_unused: usize,
) -> RunEntry {
    let mut records = Vec::new();
    for _ in 0..n_used {
        records.push(RecordRow {
            kind: "tool_result".into(),
            signals: Signals {
                id_overlap: 0.9,
                recency: 0.5,
                ..Signals::default()
            },
            utility: Utility::Used,
        });
    }
    for _ in 0..n_unused {
        records.push(RecordRow {
            kind: "tool_result".into(),
            signals: Signals {
                id_overlap: 0.1,
                recency: 0.5,
                ..Signals::default()
            },
            utility: Utility::Unused,
        });
    }
    RunEntry {
        run_id: Uuid::new_v4(),
        profile_id: profile.into(),
        tier: "degraded".into(),
        policy_version: policy.into(),
        rendered_tokens: rendered_t,
        used_tokens: used_t,
        indeterminate_share: 0.0,
        records,
    }
}

fn card(profile: &str) -> Scorecard {
    Scorecard {
        harness_version: "t".into(),
        profile_id: profile.into(),
        tier: "degraded".into(),
        hardware: "ci".into(),
        suites: vec![SuiteScore {
            suite: "honesty".into(),
            kind: SuiteKind::Replay,
            passed: 90,
            n: 100,
            flakiness: 0.0,
            attribution: harness::eval::scorecard::Attribution::default(),
            failed_cases: Vec::new(),
            sealed: false,
        }],
        cost_usd: 0.0,
        cost_tokens: 1000,
        median_steps: 4.0,
        cache_hit_rate: None,
        memory_recall: None,
        hitl_precision: None,
        utility_rate: None,
        over_estimate_rate: None,
        evaluations: 1,
        barren: vec![],
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_empty_corpus_reports_empty_and_a_run_counts_once() {
    let pool = db().await;
    assert!(
        context_corpus::refresh(&pool).await.unwrap().is_empty(),
        "nothing recorded is nothing reported, not a zero"
    );

    let e = entry("v1", "p7b", 300, 1000, 20, 20);
    assert!(context_corpus::record_run(&pool, &e).await.unwrap());
    assert!(
        !context_corpus::record_run(&pool, &e).await.unwrap(),
        "a resumed run is not counted twice"
    );

    let c = context_corpus::load(&pool, "v1").await.unwrap();
    assert_eq!(c.runs, 1);
    assert_eq!(c.observations.len(), 40);
    assert!((c.utility_rate("p7b").unwrap() - 0.3).abs() < 1e-9);
    assert_eq!(
        c.utility_rate("other"),
        None,
        "unmeasured is None, never 0.0"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_rate_is_reproducible_from_the_rows_whatever_the_insert_order() {
    let a = db().await;
    let runs = [
        entry("v1", "p", 100, 400, 10, 10),
        entry("v1", "p", 300, 600, 10, 10),
    ];
    for r in &runs {
        context_corpus::record_run(&a, r).await.unwrap();
    }
    let forward = context_corpus::load(&a, "v1")
        .await
        .unwrap()
        .utility_rate("p");

    let b = db().await;
    for r in runs.iter().rev() {
        context_corpus::record_run(&b, r).await.unwrap();
    }
    let backward = context_corpus::load(&b, "v1")
        .await
        .unwrap()
        .utility_rate("p");
    assert_eq!(forward, backward);
    assert!(
        (forward.unwrap() - 0.4).abs() < 1e-9,
        "400 used of 1000 rendered, a ratio of sums"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_thin_candidate_is_refused_and_recorded_rather_than_promoted_or_skipped() {
    let pool = db().await;
    context_corpus::record_run(&pool, &entry("v1", "p", 300, 1000, 40, 40))
        .await
        .unwrap();
    context_corpus::record_run(&pool, &entry("v2", "p", 500, 1000, 5, 5))
        .await
        .unwrap();
    let rec = context_corpus::run_promotion(&pool, "v2", &[card("p")], &[card("p")])
        .await
        .unwrap();
    assert!(!rec.promoted);
    assert!(
        rec.refused[0].contains("needed on each side"),
        "{:?}",
        rec.refused
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context_policy_promotion WHERE policy_version='v2' AND NOT promoted",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "the attempt is on the record");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_candidate_that_lost_utility_is_refused_with_its_corpus_and_a_better_one_ships() {
    let pool = db().await;
    context_corpus::record_run(&pool, &entry("v1", "p", 600, 1000, 40, 40))
        .await
        .unwrap();
    // v2 renders and uses less of it: utility 0.4 against 0.6.
    context_corpus::record_run(&pool, &entry("v2", "p", 400, 1000, 40, 40))
        .await
        .unwrap();
    let lost = context_corpus::run_promotion(&pool, "v2", &[card("p")], &[card("p")])
        .await
        .unwrap();
    assert!(!lost.promoted);
    assert!(
        lost.refused.iter().any(|r| r.contains("lost utility")),
        "{:?}",
        lost.refused
    );
    assert_eq!(lost.corpus_size, 80);

    context_corpus::record_run(&pool, &entry("v3", "p", 800, 1000, 50, 40))
        .await
        .unwrap();
    let won = context_corpus::run_promotion(&pool, "v3", &[card("p")], &[card("p")])
        .await
        .unwrap();
    assert!(won.promoted, "{:?}", won.refused);
    assert_ne!(
        won.corpus_fingerprint, lost.corpus_fingerprint,
        "a different corpus fingerprints differently"
    );
}

/// Owner-connected tests cannot see a missing GRANT; this one switches role, and
/// also proves the corpus cannot be edited under a recorded fingerprint.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_platform_role_can_write_and_read_but_not_edit_the_corpus() {
    let pool = db().await;
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO context_attribution_run (run_id, profile_id, tier, policy_version, rendered_tokens, used_tokens) \
         VALUES ($1,'p','degraded','v1',10,5)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("app_role may insert a run");
    sqlx::query(
        "INSERT INTO context_attribution_record (run_id, record_kind, signals, utility) VALUES ($1,'finding','{}','used')",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("app_role may insert a record (and use its sequence)");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM context_attribution_record")
        .fetch_one(&mut *tx)
        .await
        .expect("app_role may read");
    assert_eq!(n, 1);
    tx.commit().await.unwrap();

    let e = sqlx::query("UPDATE context_attribution_run SET used_tokens = 9")
        .execute(&pool)
        .await
        .expect_err("append-only");
    assert!(e.to_string().contains("append-only"), "{e}");
    // used cannot exceed rendered.
    assert!(sqlx::query(
        "INSERT INTO context_attribution_run (run_id, profile_id, tier, policy_version, rendered_tokens, used_tokens) \
         VALUES (gen_random_uuid(),'p','t','v1',1,2)"
    )
    .execute(&pool)
    .await
    .is_err());
}

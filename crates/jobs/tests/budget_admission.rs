//! JB-08: budget admission at submission (COMP-005 §7), against a real Postgres.
//!
//! Gated on `JOBS_BUDGET_TEST_DATABASE_URL`: a database with migrations 0001-0045
//! applied (the full set needs pgvector for 0055, which a plain PG does not have,
//! and this test needs only `users`, `research_projects` and `jobs`).
//!
//! ```bash
//! JOBS_BUDGET_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/scratch \
//!   cargo test -j 2 -p jobs --test budget_admission -- --test-threads=1
//! ```
//!
//! Only `data_qc` jobs are submitted: it is not a trial-counted kind, so the test
//! needs no experiment or ledger, and what it measures is the admission rule alone.

use std::sync::Arc;

use jobs::store::{JobStore, Submission};
use jobs::types::*;
use jobs::JobStoreError;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("JOBS_BUDGET_TEST_DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

fn store(pool: &PgPool) -> JobStore {
    JobStore::new(pool.clone(), Arc::new(jobs::PgTrialCounter))
}

/// A project with the given budgets, owned by a throwaway user.
async fn project(pool: &PgPool, compute_s: Option<i64>, usd_total: Option<f64>) -> Uuid {
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("{user}@budget.test"))
        .execute(pool)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO research_projects (project_id, user_id, kind, name, research_cutoff, \
                                        budget_compute_s, budget_usd_total) \
         VALUES ($1, $2, 'research', 'budget test', now(), $3, $4::numeric)",
    )
    .bind(id)
    .bind(user)
    .bind(compute_s)
    .bind(usd_total)
    .execute(pool)
    .await
    .unwrap();
    id
}

fn qc(project: Uuid, n: u32, compute_s: Option<f64>, usd: Option<f64>) -> Submission {
    let mut s = Submission::new(JobKind::DataQc, Uuid::new_v4(), json!({"window": n}));
    s.project_id = Some(project);
    s.submitted_by = SubmittedBy::Agent;
    s.estimate = Some(Cost {
        compute_s,
        gpu_s: None,
        cost_usd: usd,
    });
    s
}

async fn job_count(pool: &PgPool, project: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM jobs WHERE project_id = $1")
        .bind(project)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn cleanup(pool: &PgPool, project: Uuid) {
    sqlx::query("DELETE FROM jobs WHERE project_id=$1")
        .bind(project)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn an_estimate_past_the_remaining_budget_is_refused_409_and_leaves_no_row() {
    let Some(pool) = pool().await else {
        eprintln!("JOBS_BUDGET_TEST_DATABASE_URL unset - skipping");
        return;
    };
    let p = project(&pool, Some(100), None).await;
    let store = store(&pool);

    store
        .submit(qc(p, 1, Some(60.0), None))
        .await
        .expect("60 of 100");
    let err = store
        .submit(qc(p, 2, Some(60.0), None))
        .await
        .expect_err("120 of 100 must be refused");
    assert!(
        matches!(err, JobStoreError::BudgetExhausted(ref m) if m.starts_with("compute_s")),
        "{err}"
    );
    assert_eq!(
        job_count(&pool, p).await,
        1,
        "a refused submission leaves no row"
    );

    // The exact remainder fits.
    store
        .submit(qc(p, 3, Some(40.0), None))
        .await
        .expect("40 of the remaining 40");
    cleanup(&pool, p).await;
}

#[tokio::test]
async fn a_spent_budget_refuses_even_a_job_with_no_estimate() {
    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, Some(50), None).await;
    let store = store(&pool);
    store.submit(qc(p, 1, Some(50.0), None)).await.unwrap();
    let mut unpriced = qc(p, 2, None, None);
    unpriced.estimate = None;
    assert!(matches!(
        store.submit(unpriced).await,
        Err(JobStoreError::BudgetExhausted(_))
    ));
    cleanup(&pool, p).await;
}

#[tokio::test]
async fn a_project_with_no_budget_is_unlimited_and_a_system_job_is_not_budgeted() {
    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, None, None).await;
    let store = store(&pool);
    for n in 0..3 {
        store
            .submit(qc(p, n, Some(1e9), Some(1e9)))
            .await
            .expect("no limit");
    }
    cleanup(&pool, p).await;

    let mut system = Submission::new(JobKind::DataQc, Uuid::new_v4(), json!({"system": true}));
    system.estimate = Some(Cost {
        compute_s: Some(1e12),
        gpu_s: None,
        cost_usd: None,
    });
    store
        .submit(system)
        .await
        .expect("a job with no project has no budget");
}

#[tokio::test]
async fn a_duplicate_submission_spends_nothing_and_is_not_refused() {
    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, Some(100), None).await;
    let store = store(&pool);
    let first = store.submit(qc(p, 1, Some(100.0), None)).await.unwrap();
    // The budget is now fully committed, yet the identical job comes back as itself.
    let again = store.submit(qc(p, 1, Some(100.0), None)).await.unwrap();
    assert!(again.deduplicated);
    assert_eq!(again.job_id, first.job_id);
    cleanup(&pool, p).await;
}

#[tokio::test]
async fn concurrent_submissions_cannot_both_take_the_last_of_the_budget() {
    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, Some(100), None).await;
    let store = Arc::new(store(&pool));
    let mut tasks = Vec::new();
    for n in 0..10 {
        let store = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            store.submit(qc(p, n, Some(30.0), None)).await
        }));
    }
    let mut admitted = 0;
    for t in tasks {
        if t.await.unwrap().is_ok() {
            admitted += 1;
        }
    }
    assert_eq!(
        admitted, 3,
        "100 / 30 admits exactly three; the advisory lock serialises the check"
    );
    assert_eq!(job_count(&pool, p).await, 3);
    cleanup(&pool, p).await;
}

#[tokio::test]
async fn the_usd_total_budget_binds_too() {
    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, None, Some(10.0)).await;
    let store = store(&pool);
    store.submit(qc(p, 1, None, Some(6.0))).await.unwrap();
    let err = store.submit(qc(p, 2, None, Some(6.0))).await.unwrap_err();
    assert!(
        matches!(err, JobStoreError::BudgetExhausted(ref m) if m.get(..3) == Some("usd")),
        "{err}"
    );
    cleanup(&pool, p).await;
}

/// The estimate comes from the registered worker when the caller supplies none.
#[tokio::test]
async fn a_registered_worker_prices_a_submission_that_carries_no_estimate() {
    use jobs::worker::{JobContext, Worker};

    struct Priced;
    #[async_trait::async_trait]
    impl Worker for Priced {
        fn kind(&self) -> JobKind {
            JobKind::DataQc
        }
        fn estimate(&self, _m: &serde_json::Value) -> Cost {
            Cost {
                compute_s: Some(80.0),
                ..Default::default()
            }
        }
        async fn run(
            &self,
            _c: &JobContext,
            _m: &serde_json::Value,
        ) -> Result<JobOutput, JobError> {
            unreachable!("never claimed in this test")
        }
    }

    let Some(pool) = pool().await else {
        return;
    };
    let p = project(&pool, Some(100), None).await;
    let store = Arc::new(store(&pool));
    // `WorkerPool::register` is what production uses; it hands the worker to the store.
    let _pool =
        jobs::WorkerPool::new(Arc::clone(&store), WorkerClass::Data, 1).register(Arc::new(Priced));

    let mut unpriced = qc(p, 1, None, None);
    unpriced.estimate = None;
    store.submit(unpriced).await.expect("80 of 100");
    let mut second = qc(p, 2, None, None);
    second.estimate = None;
    assert!(
        matches!(
            store.submit(second).await,
            Err(JobStoreError::BudgetExhausted(_))
        ),
        "the worker's own 80s estimate pushes the second past 100"
    );
    cleanup(&pool, p).await;
}

//! Job store against a real Postgres (COMP-005 §3–§5, §10).
//!
//! The unit tests cover pure logic — hashing, clamping, kind tables. The guarantees
//! that actually matter are transactional, and only a real database can demonstrate
//! them: that a duplicate submission does not count a second trial, that a trial and
//! its job commit together or not at all, that a dead worker's job returns to the
//! queue, and that a terminal job cannot be resurrected.
//!
//! Gated on `JOBS_TEST_DATABASE_URL`. Every test works inside its own project UUID,
//! so runs do not collide, and cleans up after itself.
//!
//! ```bash
//! JOBS_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p jobs --test job_store -- --test-threads=1
//! ```

use std::sync::Arc;

use jobs::store::{JobStore, RefuseTrials, Submission, TrialCounter};
use jobs::types::*;
use jobs::JobStoreError;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("JOBS_TEST_DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.expect("connect");
    storage::postgres::run_migrations(&pool).await.expect("migrations");
    Some(pool)
}

/// Records every registration so tests can assert how many trials were counted,
/// while registering them for real in the Trial Ledger.
#[derive(Default)]
struct RecordingCounter {
    calls: std::sync::Mutex<Vec<(String, String)>>,
    fail: bool,
}

#[async_trait::async_trait]
impl TrialCounter for RecordingCounter {
    async fn register_trial(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        submission: &Submission,
        job_id: &str,
    ) -> Result<Uuid, JobStoreError> {
        if self.fail {
            return Err(JobStoreError::Invalid("counter refused".into()));
        }
        let id = jobs::PgTrialCounter.register_trial(tx, submission, job_id).await?;
        self.calls
            .lock()
            .unwrap()
            .push((submission.experiment_id.clone().unwrap_or_default(), job_id.to_string()));
        Ok(id)
    }
}

async fn cleanup(pool: &PgPool, project: Uuid) {
    sqlx::query("DELETE FROM jobs WHERE project_id=$1")
        .bind(project)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM exploration_ledger WHERE project_id=$1")
        .bind(project)
        .execute(pool)
        .await
        .ok();
}

fn submission(project: Uuid, kind: JobKind, manifest: serde_json::Value) -> Submission {
    let mut s = Submission::new(kind, Uuid::new_v4(), manifest);
    s.project_id = Some(project);
    s.submitted_by = SubmittedBy::Agent;
    if kind.counts_trial() {
        s.experiment_id = Some("exp_test".into());
    }
    s
}

#[tokio::test]
async fn identical_submissions_dedupe_and_count_one_trial() {
    let Some(pool) = pool().await else {
        eprintln!("JOBS_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let project = Uuid::new_v4();
    let counter = Arc::new(RecordingCounter::default());
    let store = JobStore::new(pool.clone(), counter.clone());

    let first = store
        .submit(submission(
            project,
            JobKind::Backtest,
            json!({"strategy": "ema"}),
        ))
        .await
        .expect("first submit");
    assert!(!first.deduplicated);

    let second = store
        .submit(submission(
            project,
            JobKind::Backtest,
            json!({"strategy": "ema"}),
        ))
        .await
        .expect("second submit");

    assert!(second.deduplicated, "JB-02: identical work is one job");
    assert_eq!(second.job_id, first.job_id);
    // SPEC §9 / §12.4 supersede Set J's "a dedup is not a trial": the second look
    // is recorded as its own `deduplicated` trial (ADR-P0-15). It spends no compute,
    // and N_eff's correlation clustering absorbs its identical return series, so the
    // look is counted without being double-penalised.
    assert_eq!(
        counter.calls.lock().unwrap().len(),
        2,
        "a deduplicated submission is still a look and is registered"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn a_rerun_nonce_makes_a_new_job_and_a_new_trial() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let counter = Arc::new(RecordingCounter::default());
    let store = JobStore::new(pool.clone(), counter.clone());

    store
        .submit(submission(project, JobKind::Backtest, json!({"s": 1})))
        .await
        .expect("first");
    let rerun = store
        .submit(submission(
            project,
            JobKind::Backtest,
            json!({"s": 1, "rerun_nonce": "n1"}),
        ))
        .await
        .expect("rerun");

    assert!(!rerun.deduplicated, "an explicit re-run is deliberate work");
    assert_eq!(
        counter.calls.lock().unwrap().len(),
        2,
        "a re-run costs a trial — that is what stops it being a free retry"
    );

    cleanup(&pool, project).await;
}

/// The load-bearing transactional property (COMP-005 §10).
#[tokio::test]
async fn a_refused_trial_rolls_the_job_back() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let counter = Arc::new(RecordingCounter {
        fail: true,
        ..Default::default()
    });
    let store = JobStore::new(pool.clone(), counter);

    let outcome = store
        .submit(submission(project, JobKind::Backtest, json!({"s": 2})))
        .await;
    assert!(outcome.is_err(), "the submission must fail");

    let count: i64 = sqlx::query("SELECT count(*) AS n FROM jobs WHERE project_id=$1")
        .bind(project)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(
        count, 0,
        "a job whose trial could not be counted must not exist: an uncounted \
         evaluation is exactly the hole INV-1 exists to close"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn trial_counted_kinds_require_an_experiment() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let mut sub = submission(project, JobKind::Backtest, json!({"s": 3}));
    sub.experiment_id = None;

    match store.submit(sub).await {
        Err(JobStoreError::ExperimentRequired(kind)) => assert_eq!(kind, "backtest"),
        other => panic!("expected experiment_required, got {other:?}"),
    }

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn no_evaluation_service_means_no_silent_uncounted_work() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    // The default counter refuses everything rather than skipping the count.
    let store = JobStore::new(pool.clone(), Arc::new(RefuseTrials));

    assert!(
        store
            .submit(submission(project, JobKind::Backtest, json!({"s": 4})))
            .await
            .is_err(),
        "refusing to count must refuse the work, not proceed uncounted"
    );

    // A kind that does not count a trial is unaffected.
    assert!(store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 5})))
        .await
        .is_ok());

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn claim_lease_heartbeat_and_complete() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 6})))
        .await
        .expect("submit");

    let claimed = store
        .claim(WorkerClass::Research, "worker-1")
        .await
        .expect("claim")
        .expect("a queued job exists");
    assert_eq!(claimed.job_id, submitted.job_id);
    assert_eq!(claimed.state, JobState::Running);

    assert!(store
        .heartbeat(&submitted.job_id, "worker-1")
        .await
        .expect("heartbeat"));

    // A different worker cannot renew someone else's lease.
    assert!(
        !store
            .heartbeat(&submitted.job_id, "worker-2")
            .await
            .expect("foreign heartbeat"),
        "a lease belongs to one worker"
    );

    store
        .complete(
            &submitted.job_id,
            JobOutput {
                summary: Some("done".into()),
                result: json!({"sharpe": 0.1}),
                ..Default::default()
            },
        )
        .await
        .expect("complete");

    let done = store.get(&submitted.job_id).await.expect("get");
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!(done.result_summary.as_deref(), Some("done"));

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn a_claimed_job_is_not_claimed_twice() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 7})))
        .await
        .expect("submit");

    let first = store.claim(WorkerClass::Research, "w1").await.unwrap();
    assert!(first.is_some());

    // Any further claim must not return the same job. Other tests may leave work
    // around, so assert on identity rather than emptiness.
    let second = store.claim(WorkerClass::Research, "w2").await.unwrap();
    if let (Some(a), Some(b)) = (&first, &second) {
        assert_ne!(a.job_id, b.job_id, "two workers claimed the same job");
    }

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn a_worker_class_only_claims_its_own_kinds() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::DataQc, json!({"s": 8})))
        .await
        .expect("submit");

    // DataQc belongs to the data class; a trainer pool must never pick it up,
    // because it would lease a job it cannot run and stall it for a lease period.
    let wrong = store.claim(WorkerClass::Trainer, "t1").await.unwrap();
    assert!(
        wrong.map(|j| j.job_id) != Some(submitted.job_id.clone()),
        "a trainer claimed a data job"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn an_expired_lease_returns_the_job_to_the_queue() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 9})))
        .await
        .expect("submit");
    store
        .claim(WorkerClass::Research, "doomed-worker")
        .await
        .expect("claim");

    // Simulate the worker dying: the lease simply stops being renewed.
    sqlx::query("UPDATE jobs SET lease_expires_at = now() - interval '1 minute' WHERE job_id=$1")
        .bind(&submitted.job_id)
        .execute(&pool)
        .await
        .unwrap();

    let reaped = store.reap_expired_leases().await.expect("reap");
    assert!(reaped >= 1);

    let job = store.get(&submitted.job_id).await.expect("get");
    assert_eq!(
        job.state,
        JobState::Queued,
        "JB-04: a job outlives the worker that was running it"
    );
    assert_eq!(job.attempts, 1);

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn repeated_worker_loss_fails_the_job_rather_than_looping() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 10})))
        .await
        .expect("submit");

    for _ in 0..3 {
        if store
            .claim(WorkerClass::Research, "flaky")
            .await
            .unwrap()
            .is_none()
        {
            break;
        }
        sqlx::query(
            "UPDATE jobs SET lease_expires_at = now() - interval '1 minute' WHERE job_id=$1",
        )
        .bind(&submitted.job_id)
        .execute(&pool)
        .await
        .unwrap();
        store.reap_expired_leases().await.unwrap();
    }

    let job = store.get(&submitted.job_id).await.expect("get");
    assert_eq!(
        job.state,
        JobState::Failed,
        "an unrunnable job must stop consuming the queue"
    );
    let error = job.error.expect("error recorded");
    assert_eq!(error["code"], "lost_worker");

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn logic_failures_do_not_retry() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 11})))
        .await
        .expect("submit");
    store.claim(WorkerClass::Research, "w").await.unwrap();

    let state = store
        .fail(
            &submitted.job_id,
            JobError::logic(ledger::TerminalReason::IntegrityRejected, "bad_strategy", "fix the expression"),
        )
        .await
        .expect("fail");

    assert_eq!(
        state,
        JobState::Failed,
        "JB-07: a strategy that does not compile will not compile on the second try \
         either — retrying it just burns the queue"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn terminal_jobs_cannot_be_resurrected() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 12})))
        .await
        .expect("submit");
    store.claim(WorkerClass::Research, "w").await.unwrap();
    store
        .complete(&submitted.job_id, JobOutput::default())
        .await
        .expect("complete");

    // Straight past the API, as a stray SQL statement or a future bug would.
    let attempt = sqlx::query("UPDATE jobs SET state='queued' WHERE job_id=$1")
        .bind(&submitted.job_id)
        .execute(&pool)
        .await;

    assert!(
        attempt.is_err(),
        "the database must refuse to move a terminal job: a resurrected job would \
         re-run work whose trial was already counted"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn cancelling_a_parent_cancels_its_children() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let parent = store
        .submit(submission(
            project,
            JobKind::ResearchRun,
            json!({"sweep": 1}),
        ))
        .await
        .expect("parent");

    for index in 0..3 {
        let mut child = submission(project, JobKind::ResearchRun, json!({"member": index}));
        child.parent_job_id = Some(parent.job_id.clone());
        child.member_index = Some(index);
        store.submit(child).await.expect("child");
    }

    store.cancel(&parent.job_id).await.expect("cancel");

    let children: i64 =
        sqlx::query("SELECT count(*) AS n FROM jobs WHERE parent_job_id=$1 AND state='cancelled'")
            .bind(&parent.job_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");

    assert_eq!(
        children, 3,
        "children must die with their parent, or a cancelled sweep keeps burning compute"
    );

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn children_are_not_deduplicated_against_each_other() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let parent = store
        .submit(submission(
            project,
            JobKind::ResearchRun,
            json!({"sweep": 2}),
        ))
        .await
        .expect("parent");

    // Two members with an identical manifest — a sweep may legitimately evaluate the
    // same parameters under different seeds, and deduplicating them would silently
    // shrink the study.
    let mut ids = vec![];
    for index in 0..2 {
        let mut child = submission(project, JobKind::ResearchRun, json!({"same": true}));
        child.parent_job_id = Some(parent.job_id.clone());
        child.member_index = Some(index);
        ids.push(store.submit(child).await.expect("child").job_id);
    }
    assert_ne!(ids[0], ids[1], "sweep members must stay distinct");

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn events_are_recorded_and_resumable() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter::default()));

    let submitted = store
        .submit(submission(project, JobKind::ResearchRun, json!({"s": 13})))
        .await
        .expect("submit");
    store.claim(WorkerClass::Research, "w").await.unwrap();
    store
        .complete(&submitted.job_id, JobOutput::default())
        .await
        .unwrap();

    let events = store
        .events_after(0, Some(project), 100)
        .await
        .expect("events");
    let kinds: Vec<&str> = events.iter().map(|(_, _, k, _)| k.as_str()).collect();
    assert!(kinds.contains(&"state"), "state changes must be observable");
    assert!(events.len() >= 3, "queued, running, succeeded: {events:?}");

    // Resuming after the first id must not replay it (JB-06).
    let first_id = events[0].0;
    let rest = store
        .events_after(first_id, Some(project), 100)
        .await
        .expect("resume");
    assert!(rest.iter().all(|(id, _, _, _)| *id > first_id));

    cleanup(&pool, project).await;
}

#[tokio::test]
async fn exploration_is_logged_and_never_counted() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let counter = Arc::new(RecordingCounter::default());
    let store = JobStore::new(pool.clone(), counter.clone());

    store
        .log_exploration(jobs::ExplorationEntry {
            project_id: project,
            user_id: Uuid::new_v4(),
            session_id: None,
            source: "data_api",
            instruments: vec!["BTC-USD".into()],
            timeframe: Some("1m".into()),
            window_start: None,
            window_end: None,
            variables: vec!["close".into(), "volume".into()],
            description: "looked at BTC minute bars".into(),
            handle: None,
        })
        .await
        .expect("log");

    assert_eq!(store.exploration_count(project).await.unwrap(), 1);
    assert_eq!(
        counter.calls.lock().unwrap().len(),
        0,
        "D-13: exploration is reported, not counted — looking at data is not a trial"
    );

    cleanup(&pool, project).await;
}

// ---------------------------------------------------------------------------
// The real counter, against the Trial Ledger
// ---------------------------------------------------------------------------

async fn ledger_trials(pool: &PgPool, user: Uuid, config_hash: &str) -> Vec<(Uuid, String)> {
    let mut tx = ledger::pg::tenant_tx(pool, &user.to_string()).await.unwrap();
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT trial_id, state FROM mlops.trial_state WHERE tenant_id = $1 AND config_hash = $2 ORDER BY registered_at",
    )
    .bind(user.to_string())
    .bind(config_hash)
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rows
}

/// A counted job registers a trial before its row exists, and a duplicate
/// submission is a second look: a second trial, settled `deduplicated`, no second job.
#[tokio::test]
async fn a_counted_job_registers_and_a_duplicate_is_a_second_look() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(jobs::PgTrialCounter));

    let mut sub = submission(project, JobKind::Backtest, json!({"s": 100}));
    sub.experiment_id = Some("exp_ledger".into());
    let user = sub.user_id;
    let hash = sub.manifest_hash();
    let first = store.submit(sub.clone()).await.expect("submit");
    let job = store.get(&first.job_id).await.unwrap();
    assert!(job.trial_id.is_some(), "a counted job carries its trial");

    let again = store.submit(sub).await.expect("resubmit");
    assert!(again.deduplicated);
    let trials = ledger_trials(&pool, user, &hash).await;
    assert_eq!(trials.len(), 2, "asking the same question twice is two looks");
    assert_eq!(trials[0].1, "queued");
    assert_eq!(trials[1].1, "deduplicated");

    cleanup(&pool, project).await;
}

/// The job lifecycle lands in the trial's events: running, then a terminal state
/// with censoring.
#[tokio::test]
async fn job_lifecycle_settles_the_trial() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(jobs::PgTrialCounter));

    // Trainer-class work declares its own GPU ceiling; there is no default
    // (SPEC §8, AT-66).
    let mut sub = submission(project, JobKind::Train, json!({"s": 101, "max_gpu_hours": 1.0}));
    sub.experiment_id = Some("exp_ledger".into());
    let user = sub.user_id;
    let hash = sub.manifest_hash();
    let submitted = store.submit(sub).await.unwrap();
    let claimed = loop {
        match store.claim(WorkerClass::Trainer, "w1").await.unwrap() {
            Some(j) if j.job_id == submitted.job_id => break j,
            Some(other) => { store.cancel(&other.job_id).await.ok(); }
            None => panic!("job not claimable"),
        }
    };
    assert_eq!(ledger_trials(&pool, user, &hash).await[0].1, "running");
    store
        // The reason comes from the failure's `terminal` field, not from its
        // code: the code here is deliberately unhelpful, and the ledger still
        // records `oom` (AT-60).
        .fail(
            &claimed.job_id,
            JobError::terminal(ledger::TerminalReason::Oom, "worker_died", "out of memory"),
        )
        .await
        .unwrap();
    let trials = ledger_trials(&pool, user, &hash).await;
    assert_eq!(trials[0].1, "failed");
    let mut tx = ledger::pg::tenant_tx(&pool, &user.to_string()).await.unwrap();
    let (cens, reason): (String, Option<String>) = sqlx::query_as("SELECT censoring, terminal_reason FROM mlops.trial_state WHERE trial_id=$1")
        .bind(trials[0].0).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(cens, "failed");
    assert_eq!(reason.as_deref(), Some("oom"));

    cleanup(&pool, project).await;
}

/// A refused registration leaves no job behind.
#[tokio::test]
async fn a_refused_registration_leaves_no_job() {
    let Some(pool) = pool().await else { return };
    let project = Uuid::new_v4();
    let store = JobStore::new(pool.clone(), Arc::new(RecordingCounter { fail: true, ..Default::default() }));

    let mut sub = submission(project, JobKind::Backtest, json!({"s": 102}));
    sub.experiment_id = Some("exp_x".into());
    assert!(store.submit(sub).await.is_err());

    let count: i64 = sqlx::query("SELECT count(*) AS n FROM jobs WHERE project_id=$1")
        .bind(project)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(count, 0, "no job may exist whose trial went nowhere");

    cleanup(&pool, project).await;
}

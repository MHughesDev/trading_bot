//! Mid-run checkpoints and the resume decision, through the job service
//! (SPEC §9, checklist 2.4/2.5, ADR-P2-07, AT-61).
//!
//! `jobs::checkpoint::plan_resume` was complete and tested and had no caller, and
//! no adapter wrote a checkpoint mid-run, so a re-queued job always restarted from
//! step 0 even when its framework had been *proved* bit-identical on resume. This
//! exercises the two halves the service now provides: `JobContext::write_checkpoint`
//! (the adapter's) and `JobContext::resume_plan` (the worker's).
//!
//! Gated on `JOBS_TEST_DATABASE_URL`, like `job_store.rs`, but applying no
//! migrations itself.
//!
//! ```bash
//! JOBS_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p jobs --test checkpoint_resume -- --test-threads=1
//! ```

use std::sync::{Arc, Mutex};

use jobs::checkpoint::ResumePlan;
use jobs::store::{JobStore, RefuseTrials, Submission};
use jobs::types::*;
use jobs::{JobContext, JobError, JobOutput, Worker, WorkerPool};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("JOBS_TEST_DATABASE_URL").ok()?;
    // The database is expected to be migrated already (through 0037 at least);
    // this test applies nothing, so it can run against a scratch database that
    // was built with `psql` rather than `sqlx migrate`.
    Some(PgPool::connect(&url).await.expect("connect"))
}

fn manifest(step: u64, code: &str) -> Value {
    json!({
        "step": step,
        "rng_state": { "python": "a", "numpy": "b", "torch_cpu": "c", "torch_cuda": [] },
        "optimizer_state_ref": format!("art_opt_{step}"),
        "lr_scheduler_state_ref": "art_sched",
        "dataloader_position": 128,
        "amp_scaler_state": "art_scaler",
        "code_hash": code,
        "image_digest": "sha256:image",
        "dataset_id": "ds:abc",
        "resume_support": "bit_identical"
    })
}

/// A worker that does what a training adapter would: ask how to pick up, record
/// what it was told, then write a checkpoint as it "trains".
struct Resuming {
    seen: Arc<Mutex<Vec<ResumePlan>>>,
    write_step: Option<u64>,
    code: &'static str,
}

#[async_trait::async_trait]
impl Worker for Resuming {
    fn kind(&self) -> JobKind {
        // A kind that counts no trial, so this needs no ledger registration.
        JobKind::DataQc
    }

    async fn run(&self, ctx: &JobContext, _manifest: &Value) -> Result<JobOutput, JobError> {
        let plan = ctx
            .resume_plan(self.code, "sha256:image", "ds:abc")
            .await
            .map_err(|e| JobError::infrastructure(e.to_string()))?;
        self.seen.lock().unwrap().push(plan);
        if let Some(step) = self.write_step {
            ctx.write_checkpoint(&manifest(step, self.code))
                .await
                .map_err(|e| JobError::infrastructure(e.to_string()))?;
        }
        Ok(JobOutput::default())
    }
}

async fn submit_and_claim(store: &Arc<JobStore>, project: Uuid) -> jobs::store::Job {
    let mut s = Submission::new(JobKind::DataQc, Uuid::new_v4(), json!({ "p": project }));
    s.project_id = Some(project);
    store.submit(s).await.expect("submit");
    store
        .claim(WorkerClass::Data, "test-worker")
        .await
        .expect("claim")
        .expect("a queued job")
}

/// A first attempt has no checkpoint and is told to start; the checkpoint it
/// writes is what the *next* attempt is told to resume from, under the same job.
#[tokio::test]
async fn a_written_checkpoint_is_what_a_re_queued_job_resumes_from() {
    let Some(pool) = pool().await else {
        eprintln!("JOBS_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let store = Arc::new(JobStore::new(pool.clone(), Arc::new(RefuseTrials)));
    let project = Uuid::new_v4();
    let seen = Arc::new(Mutex::new(Vec::new()));

    let job = submit_and_claim(&store, project).await;
    let first =
        WorkerPool::new(Arc::clone(&store), WorkerClass::Data, 1).register(Arc::new(Resuming {
            seen: Arc::clone(&seen),
            write_step: Some(900),
            code: "sha256:code",
        }));
    first.execute(job.clone()).await;

    // The same job, picked up again (a lost lease re-queues it; the checkpoint
    // is keyed by job, not by attempt).
    let second =
        WorkerPool::new(Arc::clone(&store), WorkerClass::Data, 1).register(Arc::new(Resuming {
            seen: Arc::clone(&seen),
            write_step: None,
            code: "sha256:code",
        }));
    second.execute(job.clone()).await;

    let plans = seen.lock().unwrap().clone();
    assert!(
        matches!(plans[0], ResumePlan::Restart { .. }),
        "no checkpoint yet: {:?}",
        plans[0]
    );
    assert_eq!(
        plans[1],
        ResumePlan::Resume {
            step: 900,
            from: "art_opt_900".into()
        },
        "the second attempt resumes from what the first wrote"
    );

    sqlx::query("DELETE FROM jobs WHERE project_id=$1")
        .bind(project)
        .execute(&pool)
        .await
        .ok();
}

/// A checkpoint from different code is not this computation's: restart, under
/// the same trial, with the reason.
#[tokio::test]
async fn a_checkpoint_from_other_code_restarts_with_a_reason() {
    let Some(pool) = pool().await else {
        eprintln!("JOBS_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let store = Arc::new(JobStore::new(pool.clone(), Arc::new(RefuseTrials)));
    let project = Uuid::new_v4();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let job = submit_and_claim(&store, project).await;

    WorkerPool::new(Arc::clone(&store), WorkerClass::Data, 1)
        .register(Arc::new(Resuming {
            seen: Arc::clone(&seen),
            write_step: Some(10),
            code: "sha256:old",
        }))
        .execute(job.clone())
        .await;
    WorkerPool::new(Arc::clone(&store), WorkerClass::Data, 1)
        .register(Arc::new(Resuming {
            seen: Arc::clone(&seen),
            write_step: None,
            code: "sha256:new",
        }))
        .execute(job.clone())
        .await;

    match &seen.lock().unwrap()[1] {
        ResumePlan::Restart { reason } => {
            assert!(reason.contains("different computation"), "{reason}");
            assert!(reason.contains("same trial"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    sqlx::query("DELETE FROM jobs WHERE project_id=$1")
        .bind(project)
        .execute(&pool)
        .await
        .ok();
}

/// A partial checkpoint never gets in: the store refuses it by field name, so
/// `plan_resume` never meets one it would have to `Abandon`.
#[tokio::test]
async fn a_partial_checkpoint_is_refused_by_name_and_not_stored() {
    let Some(pool) = pool().await else {
        eprintln!("JOBS_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let store = Arc::new(JobStore::new(pool.clone(), Arc::new(RefuseTrials)));
    let project = Uuid::new_v4();
    let job = submit_and_claim(&store, project).await;

    for field in [
        "step",
        "rng_state",
        "optimizer_state_ref",
        "dataloader_position",
        "code_hash",
    ] {
        let mut m = manifest(5, "sha256:code");
        m.as_object_mut().unwrap().remove(field);
        let err = store.record_checkpoint(&job.job_id, &m).await.unwrap_err();
        assert!(
            err.to_string().contains("checkpoint_invalid") && err.to_string().contains(field),
            "{field}: {err}"
        );
    }
    assert_eq!(
        store.latest_checkpoint(&job.job_id).await.unwrap(),
        None,
        "nothing partial was stored"
    );

    // The newest wins when there are several.
    store
        .record_checkpoint(&job.job_id, &manifest(5, "sha256:code"))
        .await
        .unwrap();
    store
        .record_checkpoint(&job.job_id, &manifest(9, "sha256:code"))
        .await
        .unwrap();
    let latest = store.latest_checkpoint(&job.job_id).await.unwrap().unwrap();
    assert_eq!(latest["step"], 9);

    sqlx::query("DELETE FROM jobs WHERE project_id=$1")
        .bind(project)
        .execute(&pool)
        .await
        .ok();
}

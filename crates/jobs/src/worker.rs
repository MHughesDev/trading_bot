//! Worker SDK and the runner loop (COMP-005 §12, JB-04, JB-05).
//!
//! A worker is a pure function of its manifest. Given the same manifest, the same
//! pinned data snapshot and the same code hashes, it must produce the same result —
//! which is what makes a job safe to re-queue after a lost lease, and what makes
//! `manifest_hash` a meaningful identity rather than a cache key that happens to
//! work most of the time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracing::{debug, error, info, warn};

use crate::store::{Job, JobStore, LEASE_SECONDS};
use crate::types::*;

/// What a worker can do while running.
pub struct JobContext {
    pub job_id: String,
    /// Proof of registration for trial-counted kinds. A counted job never reaches a
    /// worker without one (INV-16).
    pub trial: Option<ledger::TrialTicket>,
    pub project_id: Option<uuid::Uuid>,
    store: Arc<JobStore>,
    cancelled: Arc<AtomicBool>,
    last_progress: std::sync::Mutex<std::time::Instant>,
}

impl JobContext {
    /// Reports progress, rate-limited to one update per 5 s (COMP-005 §6).
    ///
    /// The limit is enforced here rather than asked of callers: a training loop that
    /// reported every epoch would bury the event stream, and the agent reading it
    /// pays per token for the noise.
    pub async fn progress(&self, progress: Progress) {
        {
            let mut last = self.last_progress.lock().expect("progress mutex");
            if last.elapsed() < Duration::from_secs(5) {
                return;
            }
            *last = std::time::Instant::now();
        }
        if let Err(e) = self.store.report_progress(&self.job_id, &progress).await {
            debug!(job = %self.job_id, error = %e, "progress update failed");
        }
    }

    /// Whether cancellation has been requested. Long-running workers must poll this.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Implemented per job kind.
#[async_trait::async_trait]
pub trait Worker: Send + Sync {
    fn kind(&self) -> JobKind;

    /// Predicted cost, used for budget admission before the job is queued.
    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost::default()
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError>;
}

/// Runs registered workers for one worker class, claiming jobs as they appear.
pub struct WorkerPool {
    store: Arc<JobStore>,
    class: WorkerClass,
    worker_id: String,
    workers: HashMap<JobKind, Arc<dyn Worker>>,
    max_parallel: usize,
}

impl WorkerPool {
    pub fn new(store: Arc<JobStore>, class: WorkerClass, max_parallel: usize) -> Self {
        Self {
            store,
            class,
            worker_id: format!("{}-{}", class.as_str(), uuid::Uuid::new_v4()),
            workers: HashMap::new(),
            max_parallel: max_parallel.max(1),
        }
    }

    /// Registers a worker. Its kind must belong to this pool's class, otherwise the
    /// pool would claim jobs it cannot run and stall them behind a lease.
    pub fn register(mut self, worker: Arc<dyn Worker>) -> Self {
        let kind = worker.kind();
        assert_eq!(
            kind.worker_class(),
            self.class,
            "worker for {kind} belongs to the {:?} class, not {:?}",
            kind.worker_class(),
            self.class
        );
        self.workers.insert(kind, worker);
        self
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Claims and runs jobs until the process stops.
    ///
    /// Concurrency is bounded by a semaphore rather than by a fixed cap inside any
    /// one subsystem — the fixed 3-concurrent backtest semaphore this replaces was
    /// invisible to everything else and could not be tuned per deployment (JB-05).
    pub async fn run_forever(self: Arc<Self>) {
        let permits = Arc::new(tokio::sync::Semaphore::new(self.max_parallel));
        info!(
            class = self.class.as_str(),
            worker = %self.worker_id,
            max_parallel = self.max_parallel,
            kinds = self.workers.len(),
            "worker pool started"
        );

        loop {
            let permit = match Arc::clone(&permits).acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let claimed = match self.store.claim(self.class, &self.worker_id).await {
                Ok(job) => job,
                Err(e) => {
                    warn!(class = self.class.as_str(), error = %e, "claim failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            let Some(job) = claimed else {
                drop(permit);
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            };

            let this = Arc::clone(&self);
            tokio::spawn(async move {
                this.execute(job).await;
                drop(permit);
            });
        }
    }

    /// Runs one job to completion, heartbeating alongside it.
    pub async fn execute(&self, job: Job) {
        let Some(worker) = self.workers.get(&job.kind).cloned() else {
            // Nothing is wrong with the request; the thing it needs is absent.
            let error = JobError::logic(
                ledger::TerminalReason::DependencyFailure,
                "no_worker",
                format!("this pool has no worker registered for {}", job.kind),
            );
            let _ = self.store.fail(&job.job_id, error).await;
            return;
        };

        let trial = if job.kind.counts_trial() {
            let Some(trial_id) = job.trial_id else {
                let error = JobError::logic(ledger::TerminalReason::IntegrityRejected, "unregistered_trial", format!("{} jobs dispatch compute and need a registered trial (INV-16)", job.kind));
                let _ = self.store.fail(&job.job_id, error).await;
                return;
            };
            match ledger::pg::PgTrialLedger::new(self.store.pool().clone()).resume_ticket_async(&job.user_id.to_string(), trial_id).await {
                Ok(t) => Some(t),
                Err(e) => {
                    let error = JobError::logic(ledger::TerminalReason::IntegrityRejected, "unregistered_trial", format!("trial {trial_id} cannot authorize this job: {e}"));
                    let _ = self.store.fail(&job.job_id, error).await;
                    return;
                }
            }
        } else {
            None
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let ctx = JobContext {
            trial,
            job_id: job.job_id.clone(),
            project_id: job.project_id,
            store: Arc::clone(&self.store),
            cancelled: Arc::clone(&cancelled),
            last_progress: std::sync::Mutex::new(
                std::time::Instant::now() - Duration::from_secs(10),
            ),
        };

        // Heartbeat on a third of the lease, so two consecutive misses still leave
        // room to renew before the lease expires and the job is re-queued underneath
        // a worker that is in fact fine.
        let beat_store = Arc::clone(&self.store);
        let beat_job = job.job_id.clone();
        let beat_worker = self.worker_id.clone();
        let beat_flag = Arc::clone(&cancelled);
        let heartbeat = tokio::spawn(async move {
            let period = Duration::from_secs((LEASE_SECONDS / 3).max(1) as u64);
            loop {
                tokio::time::sleep(period).await;
                match beat_store.heartbeat(&beat_job, &beat_worker).await {
                    Ok(true) => {}
                    Ok(false) => {
                        beat_flag.store(true, Ordering::Relaxed);
                        return;
                    }
                    Err(e) => debug!(job = %beat_job, error = %e, "heartbeat failed"),
                }
            }
        });

        // The declared GPU ceiling is a wall, not a warning (SPEC §8, AT-66).
        // A job that has spent its budget is killed where it stands and settles
        // `budget_exceeded`, which INV-17 censors `right_budget` — a censored
        // observation, not a failure, because nothing went wrong with the work.
        let outcome = match gpu_budget(&job) {
            Some(limit) => {
                let deadline = Duration::from_secs_f64(limit * 3600.0);
                match tokio::time::timeout(deadline, worker.run(&ctx, &job.manifest)).await {
                    Ok(result) => result,
                    Err(_) => Err(JobError::terminal(
                        ledger::TerminalReason::BudgetExceeded,
                        "budget_exceeded",
                        format!(
                            "the job declared max_gpu_hours = {limit} and was stopped at the limit"
                        ),
                    )),
                }
            }
            None => worker.run(&ctx, &job.manifest).await,
        };
        heartbeat.abort();

        match outcome {
            Ok(output) => {
                if let Err(e) = self.store.complete(&job.job_id, output).await {
                    error!(job = %job.job_id, error = %e, "completing job failed");
                }
            }
            Err(error) => {
                let code = error.code.clone();
                match self.store.fail(&job.job_id, error).await {
                    Ok(state) => {
                        info!(job = %job.job_id, %code, state = state.as_str(), "job failed")
                    }
                    Err(e) => error!(job = %job.job_id, error = %e, "failing job failed"),
                }
            }
        }
    }
}

/// The GPU ceiling a `Trainer`-class job declared.
///
/// `None` for every other class — they are CPU work and the lease reaper is
/// their bound. For a trainer job the submission path has already refused a
/// manifest without one (`require_gpu_budget`), so `None` here would mean a row
/// written before that check existed; it is not treated as "unlimited", it is
/// treated as "not a trainer job", and the lease still bounds it.
fn gpu_budget(job: &Job) -> Option<f64> {
    if job.kind.worker_class() != WorkerClass::Trainer {
        return None;
    }
    job.manifest
        .get("max_gpu_hours")
        .and_then(Value::as_f64)
        .filter(|h| h.is_finite() && *h > 0.0)
}

/// Periodically re-queues jobs whose worker stopped reporting (JB-04).
pub async fn run_lease_reaper(store: Arc<JobStore>) {
    let period = Duration::from_secs((LEASE_SECONDS / 2).max(5) as u64);
    loop {
        tokio::time::sleep(period).await;
        match store.reap_expired_leases().await {
            Ok(0) => {}
            Ok(n) => warn!(reaped = n, "re-queued jobs with expired leases"),
            Err(e) => warn!(error = %e, "lease reaper failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy(JobKind);

    #[async_trait::async_trait]
    impl Worker for Dummy {
        fn kind(&self) -> JobKind {
            self.0
        }
        async fn run(&self, _ctx: &JobContext, _m: &Value) -> Result<JobOutput, JobError> {
            Ok(JobOutput::default())
        }
    }

    #[test]
    #[should_panic(expected = "belongs to the")]
    fn registering_a_worker_in_the_wrong_pool_panics() {
        // A pool that claimed jobs it could not run would lease them and then fail
        // them, which looks like a broken job rather than a misconfigured pool.
        //
        // `connect_lazy` needs a Tokio context even though it never dials, so the
        // check runs inside a runtime. Nothing here touches the database: the
        // assertion in `register` fires first.
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let pool = WorkerPool::new(
            Arc::new(JobStore::new(
                sqlx::PgPool::connect_lazy("postgres://invalid/invalid").expect("lazy pool"),
                Arc::new(crate::store::RefuseTrials),
            )),
            WorkerClass::Eval,
            1,
        );
        let _ = pool.register(Arc::new(Dummy(JobKind::Backtest)));
    }

    #[test]
    fn worker_classes_partition_the_kinds() {
        // Every kind belongs to exactly one class, so exactly one pool claims it.
        for kind in JobKind::ALL {
            let class = kind.worker_class();
            let matching: Vec<_> = WorkerClass::ALL.iter().filter(|c| **c == class).collect();
            assert_eq!(matching.len(), 1, "{kind} maps to {class:?}");
        }
    }
}

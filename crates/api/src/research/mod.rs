//! Research orchestration (FEAT-003 Phase 1): sweep jobs over the Backtest
//! Suite, following the platform's job pattern (insert a status row, spawn a
//! driver, poll over REST). Jobs are in-memory like the suite they drive.
//!
//! The [`SweepBackend`] implementation here is the *only* caller of
//! `SuiteManager::run_param_batch` — the sampler-facing path stays in-process
//! and is never routed over HTTP.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use uuid::Uuid;

use backtest::run::{Objective, ParamMap};
use backtest::stats::diagnostics::DiagnosticBundle;
use backtest::suite::{ParamBatchOutcome, ParamBatchSpec, SuiteManager};
use domain::strategy_def::StrategyDefinition;
use research::{
    run_sweep, Narrow, SamplerKind, SweepBackend, SweepError, SweepObserver, SweepReport,
    SweepRequest,
};

/// HTTP body for `POST /api/research/sweeps`. `strategy_ref` and `objective`
/// default to the Experiment's own.
#[derive(Clone, Debug, Deserialize)]
pub struct StartSweepBody {
    pub experiment_id: Uuid,
    #[serde(default)]
    pub strategy_ref: Option<String>,
    #[serde(default)]
    pub objective: Option<Objective>,
    #[serde(default)]
    pub narrowing: std::collections::BTreeMap<String, Narrow>,
    #[serde(default)]
    pub sampler: SamplerKind,
    #[serde(default)]
    pub max_runs: Option<u32>,
    #[serde(default)]
    pub batch_size: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    pub question: String,
    #[serde(default)]
    pub base_params: Option<ParamMap>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SweepStatus {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize)]
struct SweepState {
    status: SweepStatus,
    done: u32,
    planned: u32,
    note: String,
    report: Option<SweepReport>,
    error: Option<String>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

struct SweepJob {
    id: Uuid,
    user_id: Uuid,
    request: SweepRequest,
    created_at: DateTime<Utc>,
    state: RwLock<SweepState>,
    cancel: AtomicBool,
}

/// What the API returns for a sweep.
#[derive(Clone, Debug, Serialize)]
pub struct SweepSnapshot {
    pub sweep_id: Uuid,
    pub experiment_id: Uuid,
    pub strategy_ref: String,
    pub question: String,
    pub sampler: SamplerKind,
    pub max_runs: u32,
    pub status: SweepStatus,
    /// Sampled Runs so far / planned (the neighbourhood cube is extra).
    pub done: u32,
    pub planned: u32,
    pub note: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub report: Option<SweepReport>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("experiment not found")]
    ExperimentNotFound,
    #[error("research is at capacity — wait for a sweep to finish")]
    Busy,
}

pub struct ResearchManager {
    pg: PgPool,
    suite: Arc<SuiteManager>,
    jobs: RwLock<HashMap<Uuid, Arc<SweepJob>>>,
    permits: Arc<Semaphore>,
}

impl ResearchManager {
    #[must_use]
    pub fn new(pg: PgPool, suite: Arc<SuiteManager>, max_parallel: usize) -> Arc<Self> {
        Arc::new(Self {
            pg,
            suite,
            jobs: RwLock::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(max_parallel.max(1))),
        })
    }

    /// Validate against the Experiment, persist the job, spawn the sweep.
    pub fn start(
        self: &Arc<Self>,
        user_id: Uuid,
        body: StartSweepBody,
    ) -> Result<Uuid, StartError> {
        let exp = self
            .suite
            .get_experiment(user_id, body.experiment_id)
            .ok_or(StartError::ExperimentNotFound)?;
        let objective = body.objective.or(exp.objective).ok_or_else(|| {
            StartError::InvalidRequest(
                "experiment has no objective — pass `objective` or create the experiment with one"
                    .into(),
            )
        })?;
        objective.validate().map_err(StartError::InvalidRequest)?;
        if body.question.trim().is_empty() {
            return Err(StartError::InvalidRequest(
                "question must not be empty".into(),
            ));
        }
        if self.permits.available_permits() == 0 {
            return Err(StartError::Busy);
        }
        let request = SweepRequest {
            experiment_id: body.experiment_id,
            strategy_ref: body.strategy_ref.unwrap_or(exp.strategy_ref),
            objective,
            narrowing: body.narrowing,
            sampler: body.sampler,
            max_runs: body.max_runs.unwrap_or(40).clamp(1, 2_000),
            batch_size: body.batch_size.unwrap_or(8).clamp(1, 64),
            seed: body.seed.unwrap_or_else(|| Utc::now().timestamp() as u64),
            question: body.question.trim().to_string(),
            base_params: body.base_params,
        };

        let id = Uuid::new_v4();
        let job = Arc::new(SweepJob {
            id,
            user_id,
            request: request.clone(),
            created_at: Utc::now(),
            state: RwLock::new(SweepState {
                status: SweepStatus::Queued,
                done: 0,
                planned: request.max_runs,
                note: "queued".into(),
                report: None,
                error: None,
                started_at: None,
                finished_at: None,
            }),
            cancel: AtomicBool::new(false),
        });
        self.jobs
            .write()
            .expect("research jobs lock")
            .insert(id, Arc::clone(&job));

        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let permit = match Arc::clone(&manager.permits).acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            {
                let mut s = job.state.write().expect("sweep state lock");
                s.status = SweepStatus::Running;
                s.note = "running".into();
                s.started_at = Some(Utc::now());
            }
            let backend = Backend {
                pg: manager.pg.clone(),
                suite: Arc::clone(&manager.suite),
                user_id,
                handle: Handle::current(),
            };
            let observer = Observer {
                job: Arc::clone(&job),
            };
            let req = job.request.clone();
            let outcome =
                tokio::task::spawn_blocking(move || run_sweep(&backend, &req, &observer)).await;
            let mut s = job.state.write().expect("sweep state lock");
            s.finished_at = Some(Utc::now());
            match outcome {
                Ok(Ok(report)) => {
                    s.done = report.n_sampled;
                    s.status = SweepStatus::Completed;
                    s.note = format!(
                        "completed: {} runs, {} studies",
                        report.trials_consumed,
                        report.study_ids.len()
                    );
                    s.report = Some(report);
                }
                Ok(Err(SweepError::Cancelled(n))) => {
                    s.status = SweepStatus::Cancelled;
                    s.note = format!("cancelled after {n} runs");
                }
                Ok(Err(e)) => {
                    s.status = SweepStatus::Failed;
                    s.error = Some(e.to_string());
                    s.note = "failed".into();
                }
                Err(e) => {
                    s.status = SweepStatus::Failed;
                    s.error = Some(format!("sweep task panicked: {e}"));
                    s.note = "failed".into();
                }
            }
            drop(permit);
        });
        Ok(id)
    }

    fn snapshot(job: &SweepJob) -> SweepSnapshot {
        let s = job.state.read().expect("sweep state lock");
        SweepSnapshot {
            sweep_id: job.id,
            experiment_id: job.request.experiment_id,
            strategy_ref: job.request.strategy_ref.clone(),
            question: job.request.question.clone(),
            sampler: job.request.sampler,
            max_runs: job.request.max_runs,
            status: s.status,
            done: s.done,
            planned: s.planned,
            note: s.note.clone(),
            created_at: job.created_at,
            started_at: s.started_at,
            finished_at: s.finished_at,
            error: s.error.clone(),
            report: s.report.clone(),
        }
    }

    #[must_use]
    pub fn get(&self, user_id: Uuid, id: Uuid) -> Option<SweepSnapshot> {
        let jobs = self.jobs.read().expect("research jobs lock");
        jobs.get(&id)
            .filter(|j| j.user_id == user_id)
            .map(|j| Self::snapshot(j))
    }

    #[must_use]
    pub fn list(&self, user_id: Uuid) -> Vec<SweepSnapshot> {
        let jobs = self.jobs.read().expect("research jobs lock");
        let mut out: Vec<SweepSnapshot> = jobs
            .values()
            .filter(|j| j.user_id == user_id)
            .map(|j| Self::snapshot(j))
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        out
    }

    /// Request cancellation; honoured between batches.
    pub fn cancel(&self, user_id: Uuid, id: Uuid) -> bool {
        let jobs = self.jobs.read().expect("research jobs lock");
        match jobs.get(&id).filter(|j| j.user_id == user_id) {
            Some(j) => {
                j.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Diagnostics for a Run the user reached through one of their Studies.
    #[must_use]
    pub fn diagnostics(&self, user_id: Uuid, run_id: &str) -> Option<DiagnosticBundle> {
        self.suite
            .run_result(user_id, run_id)
            .map(|r| DiagnosticBundle::from_result(&r, &[]))
    }
}

/// The suite-backed [`SweepBackend`]: definitions from Postgres, batches
/// through `SuiteManager::run_param_batch` (in-process only).
struct Backend {
    pg: PgPool,
    suite: Arc<SuiteManager>,
    user_id: Uuid,
    handle: Handle,
}

impl SweepBackend for Backend {
    fn definition(&self, strategy_ref: &str) -> Result<StrategyDefinition, String> {
        let pg = self.pg.clone();
        let slug = strategy_ref.to_string();
        let row: Option<(serde_json::Value,)> = self
            .handle
            .block_on(async move {
                sqlx::query_as(
                    "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
                )
                .bind(&slug)
                .fetch_optional(&pg)
                .await
            })
            .map_err(|e| format!("strategy lookup failed: {e}"))?;
        let (json,) = row.ok_or_else(|| format!("strategy '{strategy_ref}' not found"))?;
        serde_json::from_value(json).map_err(|e| format!("invalid stored definition: {e}"))
    }

    fn run_batch(
        &self,
        experiment: Uuid,
        spec: ParamBatchSpec,
    ) -> Result<ParamBatchOutcome, String> {
        self.suite
            .run_param_batch(self.user_id, experiment, spec)
            .map_err(|e| e.to_string())
    }
}

struct Observer {
    job: Arc<SweepJob>,
}

impl SweepObserver for Observer {
    fn progress(&self, done: u32, planned: u32, note: &str) {
        let mut s = self.job.state.write().expect("sweep state lock");
        s.done = done;
        s.planned = planned;
        s.note = note.to_string();
    }
    fn cancelled(&self) -> bool {
        self.job.cancel.load(Ordering::SeqCst)
    }
}

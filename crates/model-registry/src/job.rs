use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::RwLock as StdRwLock;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::types::{ModelRunKind, ModelRunSnapshot, RunStatus};

pub(crate) struct JobState {
    pub status: RunStatus,
    pub phase: String,
    pub error: Option<String>,
    pub metrics: Option<serde_json::Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

pub(crate) struct Job {
    pub run_id: Uuid,
    pub model_id: String,
    pub user_id: Uuid,
    pub kind: ModelRunKind,
    pub created_at: DateTime<Utc>,
    pub state: StdRwLock<JobState>,
    pub cancel: AtomicBool,
    /// 0-100 integer progress for atomic reads.
    pub progress_pct: AtomicU32,
}

impl Job {
    pub fn new(
        run_id: Uuid,
        model_id: String,
        user_id: Uuid,
        kind: ModelRunKind,
        created_at: DateTime<Utc>,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            run_id,
            model_id,
            user_id,
            kind,
            created_at,
            state: StdRwLock::new(JobState {
                status: RunStatus::Queued,
                phase: "queued".to_string(),
                error: None,
                metrics: None,
                started_at: None,
                finished_at: None,
            }),
            cancel: AtomicBool::new(false),
            progress_pct: AtomicU32::new(0),
        })
    }

    pub fn snapshot(&self) -> ModelRunSnapshot {
        let state = self.state.read().expect("job state lock poisoned");
        ModelRunSnapshot {
            run_id: self.run_id,
            model_id: self.model_id.clone(),
            kind: self.kind,
            status: state.status,
            #[allow(clippy::cast_precision_loss)]
            progress: self.progress_pct.load(Ordering::Relaxed) as f32,
            phase: state.phase.clone(),
            error: state.error.clone(),
            metrics: state.metrics.clone(),
            created_at: self.created_at,
            started_at: state.started_at,
            finished_at: state.finished_at,
        }
    }

    pub fn set_phase(&self, status: RunStatus, phase: impl Into<String>) {
        let mut state = self.state.write().expect("job state lock poisoned");
        state.status = status;
        state.phase = phase.into();
        if state.started_at.is_none() && status == RunStatus::Running {
            state.started_at = Some(Utc::now());
        }
        if status.is_terminal() && state.finished_at.is_none() {
            state.finished_at = Some(Utc::now());
        }
    }

    // Phase 2 will call this from the sidecar error path.
    #[allow(dead_code)]
    pub fn fail(&self, error: impl Into<String>) {
        let mut state = self.state.write().expect("job state lock poisoned");
        state.status = RunStatus::Failed;
        state.error = Some(error.into());
        state.finished_at = Some(Utc::now());
    }
}

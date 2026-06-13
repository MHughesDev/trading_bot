//! Backtest job orchestration: lifecycle, progress, persistence.
//!
//! Jobs run as spawned tasks; the heavy simulation itself runs on the
//! blocking pool.  Live progress is derived on demand from phase + atomic
//! counters, so there is no background updater to fall behind.  Snapshots
//! are persisted to Postgres (`backtest_runs`) best-effort at every phase
//! transition; `ClickHouse` remains the only owner of market data.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use nautilus_backtest::sdk::SimulationControl;

use crate::collect::{collect_ranges, CollectorPlan};
use crate::gaps::{self, ScheduleKind};
use crate::requirements::derive_requirements;
use crate::sim::{run_simulation, InstrumentPrecisions, SimulationInputs};
use crate::store::BarStore;
use crate::types::{BacktestSnapshot, BacktestStatus, DataCoverage, ResolvedSpec, TimeframeExt};

struct JobState {
    status: BacktestStatus,
    error: Option<String>,
    failed_phase: Option<String>,
    coverage: Option<DataCoverage>,
    result: Option<serde_json::Value>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

struct Job {
    id: Uuid,
    /// Owning user (see `api::auth::BearerToken::user_id`).
    user_id: Uuid,
    spec: ResolvedSpec,
    created_at: DateTime<Utc>,
    state: StdRwLock<JobState>,
    control: Arc<SimulationControl>,
    cancel: AtomicBool,
    collected: AtomicU64,
    collect_target: AtomicU64,
}

impl Job {
    fn new(id: Uuid, user_id: Uuid, spec: ResolvedSpec, created_at: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self {
            id,
            user_id,
            spec,
            created_at,
            state: StdRwLock::new(JobState {
                status: BacktestStatus::Queued,
                error: None,
                failed_phase: None,
                coverage: None,
                result: None,
                started_at: None,
                finished_at: None,
            }),
            control: SimulationControl::new(),
            cancel: AtomicBool::new(false),
            collected: AtomicU64::new(0),
            collect_target: AtomicU64::new(0),
        })
    }

    fn snapshot(&self) -> BacktestSnapshot {
        let state = self.state.read().expect("job state lock poisoned");
        BacktestSnapshot {
            id: self.id,
            name: self.spec.name.clone(),
            strategy_slug: self.spec.definition.strategy_id.clone(),
            instrument_id: self.spec.instrument_id.clone(),
            venue_id: self.spec.venue_id.clone(),
            asset_class: self.spec.asset_class.clone(),
            timeframe: self.spec.timeframe.key().to_string(),
            start: self.spec.start,
            end: self.spec.end,
            initial_balance: self.spec.initial_balance.clone(),
            quote_currency: self.spec.quote_currency.clone(),
            auto_collect: self.spec.auto_collect,
            status: state.status,
            progress: self.live_progress(state.status, state.failed_phase.as_deref()),
            error: state.error.clone(),
            failed_phase: state.failed_phase.clone(),
            coverage: state.coverage.clone(),
            result: state.result.clone(),
            created_at: self.created_at,
            started_at: state.started_at,
            finished_at: state.finished_at,
        }
    }

    /// Percent complete derived from phase + live counters.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn live_progress(&self, status: BacktestStatus, failed_phase: Option<&str>) -> f32 {
        match status {
            BacktestStatus::Queued => 0.0,
            BacktestStatus::CheckingData => 3.0,
            BacktestStatus::CollectingData => {
                let target = self.collect_target.load(Ordering::Relaxed);
                let done = self.collected.load(Ordering::Relaxed);
                let ratio = if target == 0 {
                    0.0
                } else {
                    (done as f32 / target as f32).min(1.0)
                };
                5.0 + 40.0 * ratio
            }
            BacktestStatus::LoadingData => 47.0,
            BacktestStatus::Simulating => 50.0 + 48.0 * self.control.progress() as f32,
            BacktestStatus::Completed => 100.0,
            BacktestStatus::Failed | BacktestStatus::Cancelled => match failed_phase {
                Some("collecting_data") => 45.0,
                Some("loading_data") => 47.0,
                Some("simulating") => 50.0 + 48.0 * self.control.progress() as f32,
                _ => 0.0,
            },
        }
    }

    fn set_status(&self, status: BacktestStatus) {
        let mut state = self.state.write().expect("job state lock poisoned");
        state.status = status;
        if status == BacktestStatus::CheckingData && state.started_at.is_none() {
            state.started_at = Some(Utc::now());
        }
        if status.is_terminal() {
            state.finished_at = Some(Utc::now());
        }
    }

    fn fail(&self, phase: BacktestStatus, error: String) {
        let mut state = self.state.write().expect("job state lock poisoned");
        state.status = BacktestStatus::Failed;
        state.failed_phase = Some(phase.as_str().to_string());
        state.error = Some(error);
        state.finished_at = Some(Utc::now());
    }
}

/// Maximum number of backtest jobs that may be in their heavy (collect /
/// load / simulate) phases at once.  Creating more simply queues them: the
/// extra jobs sit in `Queued` until a permit frees up, so N concurrent
/// creates can't spawn N simultaneous full simulations and exhaust the box.
const MAX_CONCURRENT_RUNS: usize = 3;

/// Owns all backtest jobs for the platform process.
pub struct BacktestManager {
    jobs: RwLock<HashMap<Uuid, Arc<Job>>>,
    ch_url: String,
    pg: PgPool,
    http: reqwest::Client,
    hydrated: AtomicBool,
    /// Caps the number of jobs running their heavy phases concurrently.
    run_permits: Arc<tokio::sync::Semaphore>,
}

impl BacktestManager {
    pub fn new(clickhouse_url: impl Into<String>, pg: PgPool) -> Arc<Self> {
        // Collectors hit third-party REST APIs; bound both the connect and the
        // whole-request time so a hung upstream can't wedge a job forever.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            jobs: RwLock::new(HashMap::new()),
            ch_url: clickhouse_url.into(),
            pg,
            http,
            hydrated: AtomicBool::new(false),
            run_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_RUNS)),
        })
    }

    /// Creates a job owned by `user_id` and starts driving it immediately.
    pub async fn create(
        self: &Arc<Self>,
        user_id: Uuid,
        spec: ResolvedSpec,
    ) -> anyhow::Result<Uuid> {
        anyhow::ensure!(spec.start < spec.end, "start must be before end");
        anyhow::ensure!(
            !spec.instrument_id.trim().is_empty(),
            "instrument_id is required"
        );
        let balance: Decimal = spec
            .initial_balance
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid initial_balance: {e}"))?;
        anyhow::ensure!(balance > Decimal::ZERO, "initial_balance must be positive");

        let id = Uuid::new_v4();
        let job = Job::new(id, user_id, spec, Utc::now());
        self.jobs.write().await.insert(id, Arc::clone(&job));
        self.persist(&job).await;

        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager.drive(Arc::clone(&job)).await;
            manager.persist(&job).await;
        });
        Ok(id)
    }

    /// This user's jobs, newest first.
    pub async fn list(&self, user_id: Uuid) -> Vec<BacktestSnapshot> {
        self.hydrate().await;
        let jobs = self.jobs.read().await;
        let mut out: Vec<BacktestSnapshot> = jobs
            .values()
            .filter(|j| j.user_id == user_id)
            .map(|j| j.snapshot())
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        out
    }

    /// One job, but only if owned by `user_id` (otherwise `None`, so a run's
    /// existence isn't leaked across users).
    pub async fn get(&self, user_id: Uuid, id: Uuid) -> Option<BacktestSnapshot> {
        self.hydrate().await;
        self.jobs
            .read()
            .await
            .get(&id)
            .filter(|j| j.user_id == user_id)
            .map(|j| j.snapshot())
    }

    /// Requests a stop; the job lands in `Cancelled` at the next boundary.
    pub async fn stop(&self, user_id: Uuid, id: Uuid) -> anyhow::Result<()> {
        let jobs = self.jobs.read().await;
        let job = owned(&jobs, user_id, id)?;
        let status = job.state.read().expect("job state lock poisoned").status;
        anyhow::ensure!(!status.is_terminal(), "backtest already finished");
        job.cancel.store(true, Ordering::Relaxed);
        job.control.cancel();
        Ok(())
    }

    /// Removes a finished job and its persisted row.
    pub async fn delete(&self, user_id: Uuid, id: Uuid) -> anyhow::Result<()> {
        let mut jobs = self.jobs.write().await;
        let status = {
            let job = owned(&jobs, user_id, id)?;
            job.state.read().expect("job state lock poisoned").status
        };
        anyhow::ensure!(status.is_terminal(), "stop the backtest before deleting it");
        jobs.remove(&id);
        drop(jobs);
        let _ = sqlx::query("DELETE FROM backtest_runs WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pg)
            .await;
        Ok(())
    }

    /// Starts a fresh run with the same specification (same owner).
    pub async fn rerun(self: &Arc<Self>, user_id: Uuid, id: Uuid) -> anyhow::Result<Uuid> {
        let spec = {
            let jobs = self.jobs.read().await;
            owned(&jobs, user_id, id)?.spec.clone()
        };
        self.create(user_id, spec).await
    }

    // ── Job driver ───────────────────────────────────────────────────────────

    async fn drive(self: &Arc<Self>, job: Arc<Job>) {
        // Hold a run permit for the whole job so at most MAX_CONCURRENT_RUNS
        // jobs occupy the collect/load/simulate phases at once.  While waiting
        // the job stays `Queued`; a cancel requested before the permit is
        // granted is honoured immediately without ever starting work.
        let _permit = tokio::select! {
            permit = self.run_permits.clone().acquire_owned() => match permit {
                Ok(p) => p,
                Err(_) => return, // semaphore closed (shutdown)
            },
            () = wait_for_cancel(&job) => {
                job.set_status(BacktestStatus::Cancelled);
                return;
            }
        };
        if job.cancel.load(Ordering::Relaxed) {
            job.set_status(BacktestStatus::Cancelled);
            return;
        }
        if let Err((phase, error)) = self.drive_inner(&job).await {
            tracing::warn!(id = %job.id, phase = phase.as_str(), %error, "backtest failed");
            job.fail(phase, error);
        }
    }

    async fn drive_inner(self: &Arc<Self>, job: &Arc<Job>) -> Result<(), (BacktestStatus, String)> {
        let spec = &job.spec;
        let fail = |phase: BacktestStatus| move |e: anyhow::Error| (phase, e.to_string());

        // ── Phase 1: check stored data against the strategy's requirements ──
        job.set_status(BacktestStatus::CheckingData);
        self.persist(job).await;

        let requirements = derive_requirements(&spec.definition, spec.timeframe)
            .map_err(|e| (BacktestStatus::CheckingData, e.to_string()))?;
        let timeframe = requirements.timeframe;
        let schedule = ScheduleKind::for_asset_class(&spec.asset_class);
        let warmup_secs = requirements.warmup_bars * timeframe.seconds();
        let data_from = spec.start - Duration::seconds(warmup_secs as i64);

        let store = BarStore::connect(&self.ch_url);
        let counts = store
            .daily_counts(&spec.instrument_id, timeframe, data_from, spec.end)
            .await
            .map_err(fail(BacktestStatus::CheckingData))?;
        let mut coverage = gaps::analyze(data_from, spec.end, &counts, timeframe, schedule);

        job.state.write().expect("job state lock poisoned").coverage = Some(coverage.clone());

        // ── Phase 2: fill what's missing, driven by the requirements ────────
        if !coverage.missing_ranges.is_empty() && spec.auto_collect {
            job.set_status(BacktestStatus::CollectingData);
            job.collect_target.store(
                coverage.expected_bars.saturating_sub(coverage.present_bars),
                Ordering::Relaxed,
            );
            self.persist(job).await;

            let plan = CollectorPlan::for_asset_class(&spec.asset_class, &spec.instrument_id)
                .map_err(fail(BacktestStatus::CollectingData))?;
            let collected = collect_ranges(
                &self.http,
                &store,
                &plan,
                &spec.instrument_id,
                &spec.venue_id,
                timeframe,
                &coverage.missing_ranges,
                &job.collected,
                &job.cancel,
            )
            .await
            .map_err(fail(BacktestStatus::CollectingData))?;

            // Re-check coverage after the backfill.
            let counts = store
                .daily_counts(&spec.instrument_id, timeframe, data_from, spec.end)
                .await
                .map_err(fail(BacktestStatus::CollectingData))?;
            coverage = gaps::analyze(data_from, spec.end, &counts, timeframe, schedule);
            coverage.collected_bars = collected;
            job.state.write().expect("job state lock poisoned").coverage = Some(coverage.clone());
        }

        if job.cancel.load(Ordering::Relaxed) {
            job.set_status(BacktestStatus::Cancelled);
            return Ok(());
        }
        if coverage.present_bars == 0 {
            return Err((
                BacktestStatus::CheckingData,
                format!(
                    "no historical {} bars available for {} in the requested window{}",
                    timeframe.key(),
                    spec.instrument_id,
                    if spec.auto_collect {
                        " (collection found nothing)"
                    } else {
                        " — enable auto-collect to backfill"
                    }
                ),
            ));
        }

        // ── Phase 3: load bars from ClickHouse ───────────────────────────────
        job.set_status(BacktestStatus::LoadingData);
        self.persist(job).await;
        let bars = store
            .load_bars(&spec.instrument_id, timeframe, data_from, spec.end)
            .await
            .map_err(fail(BacktestStatus::LoadingData))?;
        if bars.is_empty() {
            return Err((
                BacktestStatus::LoadingData,
                "bar load returned no rows despite coverage — check ClickHouse".to_string(),
            ));
        }

        // ── Phase 4: simulate via the market_simulator SDK ───────────────────
        job.set_status(BacktestStatus::Simulating);
        self.persist(job).await;

        let initial_balance: Decimal = spec
            .initial_balance
            .parse()
            .map_err(|e| (BacktestStatus::Simulating, format!("invalid balance: {e}")))?;
        let precisions = self.instrument_precisions(&spec.instrument_id).await;
        let inputs = SimulationInputs {
            definition: spec.definition.clone(),
            instrument_id: spec.instrument_id.clone(),
            venue_id: spec.venue_id.clone(),
            asset_class: spec.asset_class.clone(),
            timeframe,
            quote_currency: spec.quote_currency.clone(),
            initial_balance,
            precisions,
            sim_start_ns: spec.start.timestamp_nanos_opt().unwrap_or(0),
            bars,
            features: requirements.features.clone(),
        };
        let control = Arc::clone(&job.control);
        let report = tokio::task::spawn_blocking(move || run_simulation(inputs, &control))
            .await
            .map_err(|e| {
                (
                    BacktestStatus::Simulating,
                    format!("simulation panicked: {e}"),
                )
            })?
            .map_err(fail(BacktestStatus::Simulating))?;

        if report.cancelled || job.cancel.load(Ordering::Relaxed) {
            job.set_status(BacktestStatus::Cancelled);
            return Ok(());
        }

        {
            let mut state = job.state.write().expect("job state lock poisoned");
            state.result = Some(report.result);
            state.status = BacktestStatus::Completed;
            state.finished_at = Some(Utc::now());
        }
        Ok(())
    }

    /// Looks up real venue precisions from the `instruments` table.
    ///
    /// Price precision is the decimal scale of the tick size; size precision is
    /// the scale of the lot size.  Returns `None` (and the simulation infers
    /// precision from the data) when the instrument is unknown or carries no
    /// usable tick/lot metadata.
    async fn instrument_precisions(&self, instrument_id: &str) -> Option<InstrumentPrecisions> {
        let inst = match storage::postgres::instruments::fetch_by_id(&self.pg, instrument_id).await
        {
            Ok(Some(inst)) => inst,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(instrument_id, error = %e, "instrument precision lookup failed");
                return None;
            }
        };
        if inst.tick_size.is_zero() || inst.lot_size.is_zero() {
            return None;
        }
        let scale = |d: Decimal| u8::try_from(d.normalize().scale()).unwrap_or(9).min(9);
        Some(InstrumentPrecisions {
            price: scale(inst.tick_size),
            size: scale(inst.lot_size),
        })
    }

    // ── Persistence (best-effort) ────────────────────────────────────────────

    async fn persist(&self, job: &Arc<Job>) {
        let snap = job.snapshot();
        let definition = serde_json::to_value(&job.spec.definition).unwrap_or_default();
        let coverage = snap
            .coverage
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok());
        let result = sqlx::query(
            "INSERT INTO backtest_runs \
             (id, name, strategy_slug, definition, instrument_id, venue_id, asset_class, \
              timeframe, start_time, end_time, initial_balance, quote_currency, auto_collect, \
              status, progress, error, failed_phase, coverage, result, created_at, started_at, finished_at, \
              user_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23) \
             ON CONFLICT (id) DO UPDATE SET \
               status = EXCLUDED.status, progress = EXCLUDED.progress, \
               error = EXCLUDED.error, failed_phase = EXCLUDED.failed_phase, \
               coverage = EXCLUDED.coverage, result = EXCLUDED.result, \
               started_at = EXCLUDED.started_at, finished_at = EXCLUDED.finished_at",
        )
        .bind(snap.id)
        .bind(&snap.name)
        .bind(&snap.strategy_slug)
        .bind(&definition)
        .bind(&snap.instrument_id)
        .bind(&snap.venue_id)
        .bind(&snap.asset_class)
        .bind(&snap.timeframe)
        .bind(snap.start)
        .bind(snap.end)
        .bind(
            snap.initial_balance
                .parse::<Decimal>()
                .unwrap_or(Decimal::ZERO),
        )
        .bind(&snap.quote_currency)
        .bind(snap.auto_collect)
        .bind(snap.status.as_str())
        .bind(snap.progress)
        .bind(&snap.error)
        .bind(&snap.failed_phase)
        .bind(&coverage)
        .bind(&snap.result)
        .bind(snap.created_at)
        .bind(snap.started_at)
        .bind(snap.finished_at)
        .bind(job.user_id)
        .execute(&self.pg)
        .await;

        if let Err(e) = result {
            tracing::warn!(id = %job.id, error = %e, "backtest persistence failed (run continues)");
        }
    }

    /// Loads finished runs from Postgres into memory once per process.
    ///
    /// Rows still marked active belong to a previous process and are surfaced
    /// as failed ("interrupted") — jobs do not survive a platform restart.
    async fn hydrate(&self) {
        if self.hydrated.swap(true, Ordering::SeqCst) {
            return;
        }
        let rows: Vec<PersistedRun> = match sqlx::query_as(
            "SELECT id, name, definition, instrument_id, venue_id, asset_class, timeframe, \
                    start_time, end_time, initial_balance, quote_currency, auto_collect, \
                    status, error, failed_phase, coverage, result, created_at, started_at, finished_at, \
                    user_id \
             FROM backtest_runs ORDER BY created_at DESC LIMIT 500",
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "could not hydrate backtest history");
                return;
            }
        };

        let mut jobs = self.jobs.write().await;
        for row in rows {
            if jobs.contains_key(&row.id) {
                continue;
            }
            let Ok(definition) = serde_json::from_value(row.definition) else {
                continue;
            };
            let Some(timeframe) =
                <domain::payloads::bar::Timeframe as TimeframeExt>::from_key(&row.timeframe)
            else {
                continue;
            };
            let mut status = BacktestStatus::from_str_loose(&row.status);
            let mut error = row.error;
            if !status.is_terminal() {
                status = BacktestStatus::Failed;
                error = Some("interrupted by platform restart".to_string());
            }
            let job = Job::new(
                row.id,
                row.user_id.unwrap_or_else(Uuid::nil),
                ResolvedSpec {
                    name: row.name,
                    definition,
                    instrument_id: row.instrument_id,
                    venue_id: row.venue_id,
                    asset_class: row.asset_class,
                    timeframe,
                    start: row.start_time,
                    end: row.end_time,
                    initial_balance: row.initial_balance.to_string(),
                    quote_currency: row.quote_currency,
                    auto_collect: row.auto_collect,
                },
                row.created_at,
            );
            {
                let mut state = job.state.write().expect("job state lock poisoned");
                state.status = status;
                state.error = error;
                state.failed_phase = row.failed_phase;
                state.coverage = row.coverage.and_then(|c| serde_json::from_value(c).ok());
                state.result = row.result;
                state.started_at = row.started_at;
                state.finished_at = row.finished_at;
            }
            jobs.insert(row.id, job);
        }
    }
}

/// Looks a job up and confirms `user_id` owns it; otherwise reports "not found"
/// so a run's existence isn't leaked across users.
fn owned(jobs: &HashMap<Uuid, Arc<Job>>, user_id: Uuid, id: Uuid) -> anyhow::Result<&Arc<Job>> {
    jobs.get(&id)
        .filter(|j| j.user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("not found"))
}

/// Resolves once the job has been asked to cancel.  Used to abandon a job that
/// is still waiting for a run permit without ever starting its work.
async fn wait_for_cancel(job: &Arc<Job>) {
    while !job.cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Row shape of `backtest_runs` for hydration.
#[derive(sqlx::FromRow)]
struct PersistedRun {
    id: Uuid,
    name: String,
    definition: serde_json::Value,
    instrument_id: String,
    venue_id: String,
    asset_class: String,
    timeframe: String,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    initial_balance: Decimal,
    quote_currency: String,
    auto_collect: bool,
    status: String,
    error: Option<String>,
    failed_phase: Option<String>,
    coverage: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    user_id: Option<Uuid>,
}

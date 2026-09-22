//! What actually runs the knowledge plane's writers (SPEC §5, checklist 3.1/3.7).
//!
//! Phase 3 shipped two writers and scheduled neither, so
//! `knowledge.asset_embedding` and `knowledge.outcome_tensor_fact` had code,
//! tests and no rows. This module is the schedule: a worker pool for the
//! knowledge class, and a submitter that asks for the work on an interval.
//!
//! ## Why a job and not a timer
//!
//! A bare `tokio::spawn` with a `sleep` inside it is the obvious shape, and it
//! is wrong here for one reason: it has no memory. Restart the process and it
//! runs again immediately, or never, depending on where the sleep was. Run two
//! copies of the platform and both do the work. Kill it halfway through a write
//! and nothing knows the pass was half-done.
//!
//! The durable job service already solves exactly this (COMP-005 §4, ADR-0030).
//! A job is identified by the canonical hash of its manifest, and submitting the
//! same manifest twice returns the job that already exists. So the schedule is
//! expressed *in the manifest*: the profile job's `as_of` is the UTC day, the
//! tensor job's is the UTC hour. Every tick inside the same bucket submits the
//! same manifest and gets the same job back; the first tick of a new bucket is
//! the only one that creates work. A killed worker loses its lease, the reaper
//! re-queues the job, and the same manifest is executed again — which is safe
//! because both writers upsert on their primary key.
//!
//! That is the whole idempotency argument, and it holds across restarts,
//! duplicate processes and mid-write kills without any of them being special
//! cases.
//!
//! ## `knowledge_time` is the bucket, not the clock
//!
//! The bucket is not merely a deduplication key — it is the horizon the work is
//! done at (INV-01). `InstrumentProfile` bounds its bars by the `as_of` in its
//! manifest and stamps the rows with it, so a job that was queued at 00:05 and
//! finally ran at 03:40 writes knowledge as of midnight, which is what it
//! actually is. A schedule that passed `now()` down would make the same job
//! produce a different answer depending on when a worker got to it, and the
//! manifest hash would then be a lie.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use jobs::{
    Cost, JobContext, JobError, JobKind, JobOutput, JobStore, Submission, SubmittedBy, Worker,
    WorkerClass, WorkerPool,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::instrument_profile::{InstrumentProfileWorker, ProfileManifest, ProfileTarget};
use crate::knowledge_jobs::TensorProjection;

/// How often the submitter looks at the clock.
///
/// Well inside the smallest bucket, so a restart that lands mid-bucket still
/// submits promptly rather than waiting out a whole period. Submitting more
/// often than the bucket changes costs one indexed lookup per tick — the
/// deduplication is a unique index, not a scan.
const TICK: Duration = Duration::from_secs(300);

/// The bar period the fingerprint is measured on, and how much history.
///
/// Hourly over ninety days is ~2 160 bars: enough for `rv_252` and the Hurst
/// exponent, short enough that an instrument listed three months ago has a
/// fingerprint at all. Both are overridable per deployment because the right
/// answer differs by asset class, and neither has a value that is right
/// everywhere.
const PROFILE_PERIOD_SECS: u32 = 3_600;
const PROFILE_LOOKBACK_DAYS: i64 = 90;

/// The system principal system-submitted jobs are attributed to.
///
/// A fixed, recognisable UUID rather than a random one per boot: the jobs table
/// deduplicates on `(project_id, manifest_hash)` and `project_id` is NULL for
/// system work, so the user id does not affect identity — but a new id every
/// restart would make the audit trail unreadable.
pub const SYSTEM_PRINCIPAL: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_5157);

/// Start the knowledge plane's pool and submitter.
///
/// Does nothing when the job service is absent, and can be turned off with
/// `KNOWLEDGE_PLANE_SCHEDULER=off` — for an operator running a second platform
/// process that should not also be writing. The switch disables the *schedule*,
/// never the idempotency: a job submitted by hand still runs, and still upserts.
pub fn spawn(pg: PgPool, clickhouse_url: String, store: Option<Arc<JobStore>>) {
    let Some(store) = store else {
        info!("no job service; the knowledge plane's writers are not scheduled");
        return;
    };
    if std::env::var("KNOWLEDGE_PLANE_SCHEDULER").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        info!("KNOWLEDGE_PLANE_SCHEDULER=off; the knowledge plane's writers are not scheduled");
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }

    let parallel = std::env::var("KNOWLEDGE_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);

    let pool = WorkerPool::new(Arc::clone(&store), WorkerClass::Knowledge, parallel)
        .register(Arc::new(InstrumentProfileWorker::new(
            pg.clone(),
            clickhouse_url,
        )))
        .register(Arc::new(TensorRebuildWorker::new(pg.clone())));
    tokio::spawn(Arc::new(pool).run_forever());

    tokio::spawn(async move {
        loop {
            if let Err(e) = submit_due(&pg, &store, Utc::now()).await {
                warn!(error = %e, "knowledge plane scheduling failed");
            }
            tokio::time::sleep(TICK).await;
        }
    });
    info!(parallel, "knowledge plane writers scheduled");
}

/// Submit whatever the current buckets call for. Idempotent by construction.
///
/// # Errors
/// Backend failures. A submission that deduplicates is not an error — it is the
/// normal case, and the usual one.
pub async fn submit_due(
    pg: &PgPool,
    store: &JobStore,
    now: DateTime<Utc>,
) -> Result<SchedulerReport, sqlx::Error> {
    let mut report = SchedulerReport::default();

    // ── the fingerprint pass, once per UTC day ───────────────────────────────
    let as_of = floor_to_day(now);
    let instruments = population(pg, as_of).await?;
    if instruments.is_empty() {
        report.no_population = true;
    } else {
        let manifest = ProfileManifest {
            as_of,
            period_secs: PROFILE_PERIOD_SECS,
            lookback_days: PROFILE_LOOKBACK_DAYS,
            window_spec: format!("{PROFILE_LOOKBACK_DAYS}d@{PROFILE_PERIOD_SECS}s"),
            instruments,
            deflate: true,
        };
        let mut submission = Submission::new(
            JobKind::InstrumentProfile,
            SYSTEM_PRINCIPAL,
            manifest.to_value(),
        );
        submission.submitted_by = SubmittedBy::System;
        match store.submit(submission).await {
            Ok(s) => {
                report.profile_job = Some(s.job_id);
                report.profile_deduplicated = s.deduplicated;
            }
            Err(e) => warn!(error = %e, "instrument profile not submitted"),
        }
    }

    // ── the tensor projection, once per UTC hour per tenant ──────────────────
    let bucket = floor_to_hour(now);
    for tenant in tenants(pg).await? {
        let mut submission = Submission::new(
            JobKind::TensorRebuild,
            SYSTEM_PRINCIPAL,
            json!({ "tenant_id": tenant, "as_of": bucket }),
        );
        submission.submitted_by = SubmittedBy::System;
        match store.submit(submission).await {
            Ok(s) => {
                report.tensor_jobs.push(s.job_id);
                if s.deduplicated {
                    report.tensor_deduplicated += 1;
                }
            }
            Err(e) => warn!(tenant, error = %e, "tensor rebuild not submitted"),
        }
    }

    Ok(report)
}

/// What one scheduling pass asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerReport {
    pub profile_job: Option<String>,
    pub profile_deduplicated: bool,
    pub tensor_jobs: Vec<String>,
    pub tensor_deduplicated: usize,
    /// No instrument had an identity row as of the horizon. Reported rather than
    /// logged away: an empty population is the difference between "the plane is
    /// up to date" and "nothing has ever been registered", and those look
    /// identical from the table.
    pub no_population: bool,
}

/// The instruments to fingerprint, as of a horizon.
///
/// Read from `dataplane.instrument_symbol` because that is where the surrogate
/// keys the knowledge plane stores actually live, and because it is bitemporal:
/// an identity that was not known at `as_of` is not part of the population as of
/// `as_of`, which is the same rule the embeddings themselves obey.
async fn population(
    pg: &PgPool,
    as_of: DateTime<Utc>,
) -> Result<Vec<ProfileTarget>, sqlx::Error> {
    let rows: Vec<(i64, i32, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT s.instrument_id, s.venue_id, v.session_model
           FROM dataplane.instrument_symbol s
           LEFT JOIN dataplane.venue v ON v.venue_id = s.venue_id
          WHERE s.knowledge_time <= $1
            AND s.valid_from <= $1
            AND (s.valid_to IS NULL OR s.valid_to > $1)
          ORDER BY s.instrument_id, s.venue_id",
    )
    .bind(as_of)
    .fetch_all(pg)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(instrument_id, venue_id, session_model)| ProfileTarget {
            instrument_id,
            venue_id,
            continuous_venue: session_model.as_deref() == Some("continuous_24_7"),
        })
        .collect())
}

/// The tenants whose ledger is projected into the tensor.
///
/// `mlops.trial` cannot answer this. It is under FORCE row-level security keyed
/// on `app.tenant_id`, so "every tenant with a trial" is a question the schema
/// deliberately refuses — you can only see a tenant's trials if you already know
/// which tenant you are. There is no tenant registry, so the list is taken from
/// the two places a tenant id is created: the `users` table, whose ids are what
/// `PgTrialCounter` stamps on every job-dispatched trial, and an explicit
/// `KNOWLEDGE_TENANTS` for tenants created another way.
///
/// This is an honest under-approximation rather than a complete one, and it
/// fails in the safe direction: a tenant nobody lists gets no projection, which
/// is visible as an empty slice of the tensor, rather than a projection of
/// somebody else's trials.
async fn tenants(pg: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let mut out: Vec<String> = sqlx::query_scalar("SELECT user_id::text FROM users WHERE active")
        .fetch_all(pg)
        .await?;
    if let Ok(extra) = std::env::var("KNOWLEDGE_TENANTS") {
        out.extend(
            extra
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string),
        );
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The UTC midnight at or before `t`.
#[must_use]
pub fn floor_to_day(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_hour(0)
        .and_then(|t| t.with_minute(0))
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// The UTC hour at or before `t`.
#[must_use]
pub fn floor_to_hour(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

// ───────────────────────────────────────────────────────────────────────────────
// The tensor projection, as a job
// ───────────────────────────────────────────────────────────────────────────────

/// Runs [`TensorProjection::rebuild`] for one tenant.
///
/// The rebuild is already idempotent — the fact table's primary key is
/// `(tenant, trial, metric)` and the projection upserts — so the job adds
/// durability rather than safety: a pass that dies halfway leaves the facts it
/// wrote, the lease expires, and the re-queued job rewrites them to the same
/// values. Nothing is lost and nothing is doubled.
pub struct TensorRebuildWorker {
    pg: PgPool,
}

impl TensorRebuildWorker {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }
}

#[async_trait::async_trait]
impl Worker for TensorRebuildWorker {
    fn kind(&self) -> JobKind {
        JobKind::TensorRebuild
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost {
            compute_s: Some(5.0),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, _ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let tenant = manifest
            .get("tenant_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    "tensor_rebuild needs a tenant_id: the projection reads one tenant's ledger \
                     and a rebuild with no tenant would read nothing under row-level security, \
                     which is indistinguishable from a tenant with no trials",
                )
            })?;

        let projection = TensorProjection::new(self.pg.clone());
        let report = projection
            .rebuild(tenant)
            .await
            .map_err(|e| JobError::infrastructure(format!("tensor_rebuild_failed: {e}")))?;
        let (ready, why) = projection
            .ready_to_fit(tenant)
            .await
            .map_err(|e| JobError::infrastructure(format!("tensor_readiness_failed: {e}")))?;

        Ok(JobOutput {
            summary: Some(format!(
                "{} facts, {} skipped without a metric, {} censored; {why}",
                report.facts_written, report.skipped_without_metric, report.censored
            )),
            result: json!({
                "facts_written": report.facts_written,
                "skipped_without_metric": report.skipped_without_metric,
                "censored": report.censored,
                "ready_to_fit": ready,
                "readiness": why,
            }),
            ..Default::default()
        }
        .clamped())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().expect("an instant")
    }

    /// Every tick inside a bucket asks for the same job, and the first tick of
    /// the next bucket asks for a new one. That is the entire schedule: there is
    /// no stored "last run" that a restart could lose, and no timer whose
    /// position matters.
    #[test]
    fn the_day_bucket_is_what_makes_a_second_tick_a_no_op() {
        let morning = floor_to_day(at("2026-09-15T00:05:00Z"));
        let evening = floor_to_day(at("2026-09-15T23:59:59Z"));
        let tomorrow = floor_to_day(at("2026-09-16T00:00:00Z"));
        assert_eq!(morning, evening);
        assert_eq!(morning, at("2026-09-15T00:00:00Z"));
        assert_ne!(morning, tomorrow);
    }

    #[test]
    fn the_hour_bucket_rounds_down_and_keeps_the_day() {
        assert_eq!(
            floor_to_hour(at("2026-09-15T13:59:59.999Z")),
            at("2026-09-15T13:00:00Z")
        );
        assert_ne!(
            floor_to_hour(at("2026-09-15T13:00:00Z")),
            floor_to_hour(at("2026-09-15T14:00:00Z"))
        );
    }

    /// Two ticks in one bucket produce one manifest hash — which is what the job
    /// store deduplicates on. If this ever stopped holding, the scheduler would
    /// create a second job for the same knowledge day, and the two would race to
    /// write two embeddings at one knowledge time.
    #[test]
    fn two_ticks_in_one_bucket_hash_to_one_job() {
        let targets = vec![ProfileTarget {
            instrument_id: 1000,
            venue_id: 4,
            continuous_venue: false,
        }];
        let of = |t: &str| {
            let m = ProfileManifest {
                as_of: floor_to_day(at(t)),
                period_secs: PROFILE_PERIOD_SECS,
                lookback_days: PROFILE_LOOKBACK_DAYS,
                window_spec: format!("{PROFILE_LOOKBACK_DAYS}d@{PROFILE_PERIOD_SECS}s"),
                instruments: targets.clone(),
                deflate: true,
            };
            jobs::manifest::hash(&m.to_value(), &[], "latest", "instrument_profile")
        };
        assert_eq!(of("2026-09-15T00:05:00Z"), of("2026-09-15T18:30:00Z"));
        assert_ne!(of("2026-09-15T00:05:00Z"), of("2026-09-16T00:05:00Z"));
    }

    /// The tensor's bucket is per tenant as well as per hour: one tenant's
    /// projection must not deduplicate onto another's.
    #[test]
    fn a_tensor_rebuild_is_per_tenant_and_per_hour() {
        let of = |tenant: &str, t: &str| {
            jobs::manifest::hash(
                &json!({ "tenant_id": tenant, "as_of": floor_to_hour(at(t)) }),
                &[],
                "latest",
                "tensor_rebuild",
            )
        };
        assert_eq!(
            of("t_a", "2026-09-15T13:05:00Z"),
            of("t_a", "2026-09-15T13:55:00Z")
        );
        assert_ne!(
            of("t_a", "2026-09-15T13:05:00Z"),
            of("t_b", "2026-09-15T13:05:00Z")
        );
        assert_ne!(
            of("t_a", "2026-09-15T13:05:00Z"),
            of("t_a", "2026-09-15T14:05:00Z")
        );
    }

    /// The knowledge pool runs exactly the two kinds this module registers. A
    /// kind that drifted into another class would be claimed by a pool with no
    /// worker for it and failed `no_worker` — and because the failure is
    /// terminal, the next tick would deduplicate onto the dead job forever.
    #[test]
    fn the_scheduled_kinds_belong_to_the_knowledge_class() {
        assert_eq!(
            JobKind::InstrumentProfile.worker_class(),
            WorkerClass::Knowledge
        );
        assert_eq!(JobKind::TensorRebuild.worker_class(), WorkerClass::Knowledge);
    }

    /// Neither writer counts a trial. The profile looks at bars and the
    /// projection reads trials that were already counted where they were
    /// submitted; counting either would inflate N_eff by however often the
    /// schedule happens to fire, which is a number about the scheduler rather
    /// than about the research.
    #[test]
    fn scheduling_the_knowledge_plane_does_not_spend_trials() {
        assert!(!JobKind::InstrumentProfile.counts_trial());
        assert!(!JobKind::TensorRebuild.counts_trial());
    }
}

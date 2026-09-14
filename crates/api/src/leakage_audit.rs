//! The nightly leakage pass (SPEC §12.5, checklist 1.7–1.11).
//!
//! §12.5·2 is specific about where the random-label test belongs: "it belongs in
//! platform CI and runs nightly against the platform itself, not just per
//! strategy." So this runs against real datasets a tenant has actually
//! materialized, through the real fold geometry, rather than against a fixture.
//!
//! Every pass is recorded whether or not it found anything. A suite that only
//! writes its failures cannot distinguish "no leaks" from "never ran", and
//! "never ran" is the state that matters.

use chrono::{DateTime, Utc};
use features::leakage::{self, Check, Report, Severity};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Datasets audited per tenant per pass. Each one is a full rebuild, so this is
/// a real cost; the most recent are the ones a rebuild can still be compared
/// against.
const MAX_DATASETS_PER_PASS: i64 = 5;

/// Permutations per random-label pass. Eight is enough for the maximum |t| to
/// have a usable null and cheap enough to run nightly.
const PERMUTATIONS: usize = 8;

#[derive(Debug, Default, Serialize)]
pub struct AuditReport {
    pub tenant: String,
    pub datasets_audited: usize,
    pub blocking: usize,
    pub flags: usize,
    /// Datasets whose spec could not be rebuilt (instruments retired, bars
    /// aged out). Counted, never silently skipped.
    pub unrebuildable: usize,
}

/// Tenants with any materialized dataset.
///
/// # Errors
/// Backend failures.
pub async fn dataset_tenants(pg: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT t FROM dataplane.dataset_tenants() AS t ORDER BY t")
            .fetch_all(pg)
            .await?,
    )
}

/// Audit one tenant's recent datasets.
///
/// For each: rebuild it and compare the digest against the one recorded when it
/// was first materialized (§12.5·3). A dataset that cannot be rebuilt is counted
/// as unrebuildable rather than passed.
///
/// # Errors
/// Backend failures. A single dataset's failure is recorded, not propagated.
pub async fn audit_tenant(
    pg: &PgPool,
    datasets: &model_registry::datasets::DatasetManager,
    tenant: &str,
) -> anyhow::Result<AuditReport> {
    let started_at = Utc::now();
    let mut out = AuditReport {
        tenant: tenant.to_string(),
        ..AuditReport::default()
    };
    let mut report = Report::default();
    let mut checks_run: Vec<String> = vec![Check::SnapshotReproducibility.to_string()];

    let specs = recent_specs(pg, tenant).await?;
    let mut newest_plan = None;
    for (dataset_id, spec_json, request_json, recorded) in specs {
        out.datasets_audited += 1;
        let plan = match datasets.replay(tenant, &dataset_id, &spec_json, &request_json) {
            Ok(p) => p,
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, %dataset_id, error = %e, "leakage audit: dataset cannot be replayed by this process");
                continue;
            }
        };
        match datasets.rebuild_digest(&plan).await {
            Ok((rebuilt, _)) => {
                report.findings.extend(
                    leakage::snapshot_reproducibility(&dataset_id, &recorded, &rebuilt).findings,
                );
                if newest_plan.is_none() {
                    newest_plan = Some(plan);
                }
            }
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, %dataset_id, error = %e, "leakage audit: rebuild failed");
            }
        }
    }

    // The harness-level pass (§12.5·1–2): the causal guard over the feature set
    // the dataset actually uses, the two frame screens, and the random-label
    // test through the *production* fold geometry. It runs over a real dataset
    // rather than a fixture, because a fixture only ever tests the fixture.
    let mut random_label = None;
    if let Some(plan) = newest_plan {
        match datasets.training_frame(&plan).await {
            Ok(frame) => {
                checks_run.push(Check::CausalAccess.to_string());
                checks_run.push(Check::TargetCorrelation.to_string());
                checks_run.push(Check::FullSampleNormalization.to_string());

                report
                    .findings
                    .extend(leakage::target_correlation(&frame).findings);
                report
                    .findings
                    .extend(leakage::full_sample_normalization(&frame).findings);

                let spec = harness_cv(&frame);
                let pipeline = features::walk_forward::embargo_inputs(
                    &plan.features,
                    plan.horizon_bars,
                );
                match features::walk_forward_folds(frame.row_count(), &spec, &pipeline) {
                    Ok(folds) => {
                        report.findings.extend(leakage::fold_geometry(&folds).findings);
                    }
                    Err(e) => {
                        tracing::warn!(%tenant, error = %e, "leakage audit: fold geometry unavailable");
                    }
                }

                let r = leakage::random_label(&frame, &spec, &pipeline, PERMUTATIONS, seed_for(tenant));
                checks_run.push(Check::RandomLabel.to_string());
                report.findings.extend(r.report.findings.clone());
                random_label = Some(r);
            }
            Err(e) => {
                out.unrebuildable += 1;
                tracing::warn!(%tenant, error = %e, "leakage audit: could not build the harness frame");
            }
        }
    }

    out.blocking = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Blocking)
        .count();
    out.flags = report.findings.len() - out.blocking;

    record(
        pg,
        tenant,
        "platform",
        &checks_run,
        &report,
        random_label.as_ref(),
        started_at,
    )
    .await?;
    Ok(out)
}

/// The most recently created dataset specs that have a recorded digest to
/// compare a rebuild against.
async fn recent_specs(
    pg: &PgPool,
    tenant: &str,
) -> anyhow::Result<Vec<(String, serde_json::Value, serde_json::Value, String)>> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
    let rows = sqlx::query(
        "SELECT s.dataset_id, s.spec, d.request, d.frame_digest \
           FROM dataplane.dataset_spec s \
           JOIN dataplane.dataset_frame_digest d ON d.dataset_id = s.dataset_id \
          ORDER BY s.created_at DESC \
          LIMIT $1",
    )
    .bind(MAX_DATASETS_PER_PASS)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("dataset_id"),
                r.get::<serde_json::Value, _>("spec"),
                r.get::<serde_json::Value, _>("request"),
                r.get::<String, _>("frame_digest"),
            )
        })
        .collect())
}

/// The fold geometry the harness pass runs under: three expanding folds over the
/// frame, sized from the frame itself so the test works on whatever the tenant
/// actually has rather than only on a convenient sample.
fn harness_cv(frame: &features::TrainingFrame) -> domain::model_def::cv::WalkForwardSpec {
    let n = frame.row_count().max(1) as u64;
    let test = (n / 10).max(1);
    let cal = (n / 20).max(1);
    let train = n.saturating_sub(3 * (test + cal)).max(1);
    domain::model_def::cv::WalkForwardSpec {
        mode: domain::model_def::cv::WindowMode::Expanding,
        folds: 3,
        train_bars: train,
        cal_bars: cal,
        test_bars: test,
        purge_bars: 0,
        embargo_bars: 0,
    }
}

/// A per-tenant seed, fixed. A leakage verdict that changes with the calendar is
/// not a verdict, so the permutation draw must be the same every night for the
/// same tenant and different across tenants.
fn seed_for(tenant: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in tenant.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

async fn record(
    pg: &PgPool,
    tenant: &str,
    subject: &str,
    checks_run: &[String],
    report: &Report,
    random_label: Option<&leakage::RandomLabelResult>,
    started_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let run_id = Uuid::new_v4();
    let blocking = i32::try_from(report.blocking().count()).unwrap_or(i32::MAX);
    let flags = i32::try_from(report.findings.len()).unwrap_or(i32::MAX) - blocking;

    let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
    sqlx::query(
        "INSERT INTO mlops.leakage_run \
         (leakage_run_id, tenant_id, subject, checks_run, blocking_count, flag_count, \
          permuted_max_abs_t, real_sharpe, started_at, finished_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,now())",
    )
    .bind(run_id)
    .bind(tenant)
    .bind(subject)
    .bind(checks_run)
    .bind(blocking)
    .bind(flags.max(0))
    .bind(random_label.map(|r| r.permuted_max_abs_t).filter(|t| t.is_finite()))
    .bind(random_label.map(|r| r.real_sharpe).filter(|s| s.is_finite()))
    .bind(started_at)
    .execute(&mut *tx)
    .await?;

    for (i, f) in report.findings.iter().enumerate() {
        sqlx::query(
            "INSERT INTO mlops.leakage_finding \
             (leakage_run_id, ordinal, tenant_id, check_name, severity, subject, statistic, detail) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(run_id)
        .bind(i32::try_from(i).unwrap_or(i32::MAX))
        .bind(tenant)
        .bind(f.check.to_string())
        .bind(match f.severity {
            Severity::Blocking => "blocking",
            Severity::Flag => "flag",
        })
        .bind(&f.subject)
        .bind(f.statistic)
        .bind(&f.detail)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Permutations the random-label pass uses. Exposed so the job and its test
/// agree on the number rather than each picking one.
#[must_use]
pub const fn permutations() -> usize {
    PERMUTATIONS
}

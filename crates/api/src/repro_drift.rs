//! `repro_drift` — the nightly reproduction job (AGENT-013 §8.4; Set S task
//! S-10.8).
//!
//! # What it does
//!
//! Every night it re-runs a small deterministic sample of the Runs this process
//! produced in the last thirty days, under their own receipts
//! ([`backtest::suite::SuiteManager::reproduce_run`]), and publishes
//! `repro_drift`: the share that changed under an **unchanged** receipt
//! ([`backtest::receipt::DriftReport`]). A reproduction spends no trial
//! (ADR-S10-18), so the job costs compute and nothing the budget counts.
//!
//! # Drift files a defect, once it is verified
//!
//! Drift under an unchanged receipt means nothing the receipt names moved and the
//! number did — the receipt is incomplete, and every result carrying one is less
//! checkable than it looked. That is an evaluator defect, and it files one
//! ([`ledger::falsification::EvaluatorDefect`]). A drift is first re-run **once
//! more**: if the second run also differs from the stored result the defect is
//! filed `verified` (the platform reproduced it twice); if the second run
//! matches, the platform is non-deterministic and the defect is filed
//! `unverified` naming that, because "it differs from the stored result on one
//! run and not the other" is a different defect and a different fix. A changed
//! receipt files nothing: somebody changed something on purpose.
//!
//! # The sample
//!
//! 2 % of the window (`NIGHTLY_SAMPLE_SHARE`), **never fewer than one** when there
//! is anything to sample (ADR-S10-19). On a young platform 2 % of ten runs is
//! zero, and a job that reproduces nothing and reports "0 % drift" is the
//! absence of a measurement dressed as health; `drift_rate` over an empty sample is
//! `None`, and the job refuses to round its way to an empty one. The choice of
//! which runs is a function of the date and the run id, so a re-run of the job
//! the same night checks the same runs and a third party can rebuild the sample.

use std::sync::Arc;

use backtest::receipt::{DriftReport, Reproduction, NIGHTLY_SAMPLE_SHARE, NIGHTLY_WINDOW_DAYS};
use backtest::suite::{ReproducibleRun, SuiteManager};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use ledger::falsification::{EvaluatorDefect, Reproduction as ReplayedFailure, Verification};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// The runs the job re-runs on `day`: those produced in the window, ordered by a
/// hash of (day, run id), truncated to the sample size.
#[must_use]
pub fn sample(
    runs: &[ReproducibleRun],
    day: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<ReproducibleRun> {
    let cutoff = now - Duration::days(i64::from(NIGHTLY_WINDOW_DAYS));
    let mut eligible: Vec<(Vec<u8>, &ReproducibleRun)> = runs
        .iter()
        .filter(|r| r.produced_at >= cutoff)
        .map(|r| {
            let mut h = Sha256::new();
            h.update(day.to_string().as_bytes());
            h.update(b"\x1f");
            h.update(r.run_id.as_bytes());
            (h.finalize().to_vec(), r)
        })
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }
    eligible.sort_by(|a, b| a.0.cmp(&b.0));
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let want = ((eligible.len() as f64 * NIGHTLY_SAMPLE_SHARE).ceil() as usize).max(1);
    eligible
        .into_iter()
        .take(want)
        .map(|(_, r)| r.clone())
        .collect()
}

/// What one night found.
#[derive(Debug, Default)]
pub struct NightlyOutcome {
    pub report: DriftReport,
    /// Defects filed this night (verified and unverified).
    pub defects_filed: Vec<Uuid>,
    /// Runs the job could not re-run, with why. Named, not dropped: a run that
    /// could not be reproduced is not one that reproduced.
    pub skipped: Vec<(String, String)>,
}

/// Re-run tonight's sample and file what it finds.
pub async fn run_once(suite: Arc<SuiteManager>, pg: &PgPool, now: DateTime<Utc>) -> NightlyOutcome {
    let runs = suite.reproducible_runs();
    let picked = sample(&runs, now.date_naive(), now);
    let mut out = NightlyOutcome::default();
    for run in picked {
        let s = Arc::clone(&suite);
        let (user, id) = (run.user_id, run.run_id.clone());
        let first = tokio::task::spawn_blocking(move || s.reproduce_run(user, &id)).await;
        let first = match first {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                out.skipped.push((run.run_id, e.to_string()));
                continue;
            }
            Err(e) => {
                out.skipped.push((run.run_id, format!("the re-run panicked: {e}")));
                continue;
            }
        };
        out.report.record(&first.reproduction);
        if let Reproduction::DriftedUnderTheSameReceipt { was, now: observed } =
            first.reproduction.clone()
        {
            // Once more, to tell a reproducible defect from a non-deterministic one.
            let s = Arc::clone(&suite);
            let (user, id) = (run.user_id, run.run_id.clone());
            let second = tokio::task::spawn_blocking(move || s.reproduce_run(user, &id))
                .await
                .ok()
                .and_then(Result::ok);
            let reproduced_twice = second.is_some_and(|r| r.reproduction.is_platform_defect());
            match file_defect(pg, &run, was, observed, reproduced_twice).await {
                Ok(Some(id)) => out.defects_filed.push(id),
                Ok(None) => {}
                Err(e) => out
                    .skipped
                    .push((run.run_id.clone(), format!("filing the defect failed: {e}"))),
            }
        }
    }
    out
}

/// File the evaluator defect for a drift, unless one is already filed against the
/// same run. `Ok(None)` means one was.
async fn file_defect(
    pg: &PgPool,
    run: &ReproducibleRun,
    was: f64,
    now: f64,
    reproduced_twice: bool,
) -> Result<Option<Uuid>, String> {
    let tenant = run.user_id.to_string();
    let already: Option<Uuid> = sqlx::query_scalar(
        "SELECT falsification_id FROM mlops.falsification
          WHERE tenant_id = $1 AND report_kind = 'evaluator_defect' AND artifact_ref = $2
          LIMIT 1",
    )
    .bind(&tenant)
    .bind(&run.run_id)
    .fetch_optional(pg)
    .await
    .map_err(|e| e.to_string())?;
    if already.is_some() {
        return Ok(None);
    }
    let seed = {
        let d = Sha256::digest(run.run_id.as_bytes());
        u64::from_be_bytes(d[..8].try_into().expect("sha256 is 32 bytes"))
    };
    let observed = if reproduced_twice {
        format!("{now} on both re-runs")
    } else {
        format!("{now} on one re-run and the stored value on the other: non-deterministic")
    };
    let replay = ReplayedFailure::replayed(
        run.run_id.clone(),
        seed,
        format!("{was} (the stored result)"),
        observed,
        vec![
            "POST /api/lifecycle/reproduce with this run id".to_string(),
            "compare the stored metrics, equity curve and trades bit for bit".to_string(),
        ],
    )
    .map_err(|e| e.to_string())?;
    let defect = EvaluatorDefect::reported(
        "backtest executor",
        "a stored result re-runs bit-identically under an unchanged receipt (AGENT-013 §8.4)",
        replay,
    )
    .verified(if reproduced_twice {
        Verification::Verified
    } else {
        Verification::Unverified
    });
    ledger::falsification_store::insert_defect(pg, &tenant, ("repro_drift", "scheduler"), &defect)
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}

/// `repro_drift` as the table records it: reproductions in the window by outcome.
///
/// # Errors
/// Backend failures.
pub async fn drift_since(pg: &PgPool, tenant: &str, days: u32) -> Result<DriftReport, String> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant).await.map_err(|e| e.to_string())?;
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT outcome, count(*) FROM mlops.reproduction
          WHERE tenant_id = $1 AND created_at > now() - make_interval(days => $2)
          GROUP BY outcome",
    )
    .bind(tenant)
    .bind(i32::try_from(days).unwrap_or(i32::MAX))
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let mut report = DriftReport::default();
    for (outcome, n) in rows {
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        report.sampled += n;
        match outcome.as_str() {
            "bit_identical" => report.bit_identical += n,
            "drifted" => report.drifted += n,
            "receipt_changed" => report.receipt_changed += n,
            _ => {}
        }
    }
    Ok(report)
}

/// Start the nightly loop. The first pass runs after one interval, not at boot: a
/// restart empties the in-memory run store, and a pass over nothing is the empty
/// sample this job refuses to report as health.
pub fn spawn(suite: Arc<SuiteManager>, pg: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        tick.tick().await;
        loop {
            tick.tick().await;
            let out = run_once(Arc::clone(&suite), &pg, Utc::now()).await;
            tracing::info!(
                sampled = out.report.sampled,
                drifted = out.report.drifted,
                receipt_changed = out.report.receipt_changed,
                defects = out.defects_filed.len(),
                skipped = out.skipped.len(),
                "repro_drift"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(n: u32, age_days: i64, now: DateTime<Utc>) -> ReproducibleRun {
        ReproducibleRun {
            user_id: Uuid::nil(),
            run_id: format!("sha256:{n:064x}"),
            produced_at: now - Duration::days(age_days),
        }
    }

    #[test]
    fn two_percent_of_a_large_window_and_never_fewer_than_one() {
        let now = Utc::now();
        let day = now.date_naive();
        let many: Vec<_> = (0..500).map(|i| run(i, 1, now)).collect();
        assert_eq!(sample(&many, day, now).len(), 10, "ceil(2% of 500)");
        let few: Vec<_> = (0..10).map(|i| run(i, 1, now)).collect();
        assert_eq!(
            sample(&few, day, now).len(),
            1,
            "2% of ten is zero and an empty sample reports health about nothing"
        );
        assert!(sample(&[], day, now).is_empty());
    }

    #[test]
    fn runs_older_than_the_window_are_not_sampled() {
        let now = Utc::now();
        let old: Vec<_> = (0..50).map(|i| run(i, 31, now)).collect();
        assert!(sample(&old, now.date_naive(), now).is_empty());
    }

    #[test]
    fn the_sample_is_a_function_of_the_day_and_the_runs() {
        let now = Utc::now();
        let day = now.date_naive();
        let runs: Vec<_> = (0..300).map(|i| run(i, 1, now)).collect();
        assert_eq!(sample(&runs, day, now), sample(&runs, day, now));
        let mut shuffled = runs.clone();
        shuffled.reverse();
        assert_eq!(sample(&runs, day, now), sample(&shuffled, day, now));
        assert_ne!(
            sample(&runs, day, now),
            sample(&runs, day.succ_opt().unwrap(), now),
            "a different night checks different runs"
        );
    }
}

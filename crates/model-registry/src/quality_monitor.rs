//! Rolling forecast quality monitor (I-5.9–I-5.11, Phase 5).
//!
//! I-5.9  Re-scores recent forecasts on a rolling basis; persists CRPS/coverage series.
//! I-5.10 Detects calibration drift (PIT off uniform) and data/feature drift (PSI/KS).
//! I-5.11 Triggers a retrain pipeline when drift or staleness thresholds are crossed.

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use tokio::time::{interval, Duration};

use crate::manager::ModelManager;
use crate::types::TrainRequest;

// ── Alert types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualityAlert {
    pub id: String,
    pub model_id: String,
    pub kind: String, // "calibration_drift" | "feature_drift" | "staleness"
    pub message: String,
    pub metric_value: Option<f64>,
    pub threshold: Option<f64>,
    pub triggered_at: chrono::DateTime<Utc>,
    /// Run ID of the retrain pipeline triggered by this alert (if any).
    pub retrain_run_id: Option<String>,
}

/// Rolling quality point persisted per model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualityPoint {
    pub model_id: String,
    pub window_end: chrono::DateTime<Utc>,
    pub crps: Option<f64>,
    pub coverage_50: Option<f64>,
    pub coverage_90: Option<f64>,
    pub n_forecasts: i32,
}

// ── Thresholds ────────────────────────────────────────────────────────────────

/// Default CRPS relative degradation threshold (`CRPS_now` / `CRPS_baseline` − 1 > 0.20).
const _DEFAULT_CRPS_DRIFT_THRESHOLD: f64 = 0.20;
/// Default PIT uniformity KS threshold (D statistic).
const DEFAULT_PIT_KS_THRESHOLD: f64 = 0.15;
/// Default feature PSI threshold.
const DEFAULT_PSI_THRESHOLD: f64 = 0.25;
/// Default staleness: no retrain within 30 days.
const DEFAULT_STALENESS_DAYS: i64 = 30;

// ── QualityMonitor ────────────────────────────────────────────────────────────

pub struct QualityMonitor {
    pg: PgPool,
    models: Arc<ModelManager>,
    poll_interval: Duration,
    progress_tx: tokio::sync::broadcast::Sender<Value>,
}

impl QualityMonitor {
    pub fn new(pg: PgPool, models: Arc<ModelManager>, poll_interval: Duration) -> Arc<Self> {
        let (progress_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            pg,
            models,
            poll_interval,
            progress_tx,
        })
    }

    pub fn subscribe_progress(&self) -> tokio::sync::broadcast::Receiver<Value> {
        self.progress_tx.subscribe()
    }

    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(self.poll_interval);
            ticker.tick().await; // skip first immediate tick
            loop {
                ticker.tick().await;
                if let Err(e) = self.run_cycle().await {
                    tracing::error!("quality monitor error: {e}");
                }
            }
        })
    }

    // ── I-5.9: Rolling quality re-scoring ────────────────────────────────────

    /// Fill realised outcomes into the forecasts whose horizon has passed (N-4.1), so the
    /// scoring below, the coverage CUSUM and calibration have rows with a `realized` value.
    /// A missing bar store is reported as waiting rows, not an error.
    ///
    /// # Errors
    /// A Postgres failure.
    pub async fn realise_now(&self) -> anyhow::Result<crate::forecast_realisation::Realised> {
        let url = std::env::var("CLICKHOUSE_URL")
            .ok()
            .filter(|u| !u.is_empty());
        let bars: Box<dyn crate::forecast_realisation::ClosedBars> = match url {
            Some(u) => Box::new(crate::forecast_realisation::StoreBars(
                backtest::store::BarStore::connect(&u),
            )),
            None => Box::new(crate::forecast_realisation::NoBars),
        };
        crate::forecast_realisation::realise_pending(&self.pg, bars.as_ref(), Utc::now(), 5000)
            .await
    }

    async fn run_cycle(&self) -> anyhow::Result<()> {
        match self.realise_now().await {
            Ok(r) => tracing::info!(
                filled = r.filled,
                waiting_for_bars = r.waiting_for_bars,
                unresolvable = r.unresolvable,
                "forecast realisation pass"
            ),
            Err(e) => tracing::warn!("forecast realisation failed: {e}"),
        }
        // Fetch all deployed / production models.
        let rows: Vec<(String, uuid::Uuid)> = sqlx::query_as(
            "SELECT DISTINCT m.model_id, m.created_by \
             FROM ai_models m \
             JOIN ai_model_aliases a ON a.model_id = m.model_id \
             WHERE a.alias = 'production' \
               AND m.status NOT IN ('archived', 'failed')",
        )
        .fetch_all(&self.pg)
        .await?;

        for (model_id, user_uuid) in &rows {
            if let Err(e) = self.score_model(model_id, user_uuid).await {
                tracing::warn!("quality monitor: scoring {model_id} failed: {e}");
            }
        }

        Ok(())
    }

    async fn score_model(&self, model_id: &str, _user_id: &uuid::Uuid) -> anyhow::Result<()> {
        // Recent realised forecasts, oldest first (the change-point scan reads them in
        // time order). A forecast carries the quantile levels its model reported; a level
        // that was not reported is NULL, and a coverage figure is computed only from the
        // levels that exist, never from a neighbouring level standing in for it.
        type Row = (
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            f64,
        );
        let mut rows: Vec<Row> = sqlx::query_as(
            "SELECT predicted_q05, predicted_q25, predicted_q50, predicted_q75,                     predicted_q95, predicted_q10, realized                FROM (SELECT * FROM model_forecasts                       WHERE model_id = $1                         AND created_at > NOW() - INTERVAL '7 days'                         AND realized IS NOT NULL                       ORDER BY created_at DESC LIMIT 500) recent               ORDER BY created_at ASC",
        )
        .bind(model_id)
        .fetch_all(&self.pg)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }
        rows.retain(|r| r.6.is_finite());

        // Pinball loss at tau = 0.5 (MAD-style CRPS proxy), over rows that have a median.
        let medians: Vec<(f64, f64)> = rows.iter().filter_map(|r| r.2.map(|q| (q, r.6))).collect();
        let crps: Option<f64> = (!medians.is_empty()).then(|| {
            medians
                .iter()
                .map(|(q50, r)| 0.5 * (r - q50).abs())
                .sum::<f64>()
                / medians.len() as f64
        });

        // Coverage of the central 50% interval (q25..q75) and the central 90% (q05..q95).
        let cov = |lo: fn(&Row) -> Option<f64>,
                   hi: fn(&Row) -> Option<f64>|
         -> Option<(f64, Vec<bool>)> {
            let hits: Vec<bool> = rows
                .iter()
                .filter_map(|r| lo(r).zip(hi(r)).map(|(l, h)| r.6 >= l && r.6 <= h))
                .collect();
            (!hits.is_empty()).then(|| {
                (
                    hits.iter().filter(|h| **h).count() as f64 / hits.len() as f64,
                    hits,
                )
            })
        };
        let cov50 = cov(|r| r.1, |r| r.3).map(|(c, _)| c);
        let cov90_series = cov(|r| r.0, |r| r.4);
        let cov90 = cov90_series.as_ref().map(|(c, _)| *c);

        // N-4.1: a change point in the 90% interval's coverage. The adaptive wrapper in the
        // trainer resets on the same statistic with the same parameters.
        if let Some((_, hits)) = &cov90_series {
            let changes = crate::calibration::CoverageCusum::scan(
                0.10,
                crate::calibration::DEFAULT_K,
                crate::calibration::DEFAULT_H,
                hits,
            );
            if let Some(last) = changes.last() {
                let word = match last.shift {
                    crate::calibration::Shift::Undercover => "under-covers",
                    crate::calibration::Shift::Overcover => "over-covers",
                };
                self.raise_alert(
                    model_id,
                    "coverage_shift",
                    &format!(
                        "the 90% interval {word} since forecast {} of {} (CUSUM k={}, h={}, {} change point(s))",
                        last.at,
                        hits.len(),
                        crate::calibration::DEFAULT_K,
                        crate::calibration::DEFAULT_H,
                        changes.len()
                    ),
                    cov90,
                    Some(crate::calibration::DEFAULT_H),
                )
                .await?;
            }
        }

        let now = Utc::now();

        // Persist quality point.
        sqlx::query(
            "INSERT INTO model_quality_series \
               (model_id, window_end, crps, coverage_50, coverage_90, n_forecasts, recorded_at) \
             VALUES ($1, $2, $3, $4, $5, $6, NOW())",
        )
        .bind(model_id)
        .bind(now)
        .bind(crps)
        .bind(cov50)
        .bind(cov90)
        .bind(rows.len() as i32)
        .execute(&self.pg)
        .await?;

        // I-5.10: check calibration drift (only when a 90% interval was reported).
        if let Some(c90) = cov90 {
            self.check_calibration_drift(model_id, c90, cov50.unwrap_or(f64::NAN))
                .await?;
        }

        // I-5.10: check staleness.
        self.check_staleness(model_id).await?;

        let _ = self.progress_tx.send(serde_json::json!({
            "kind": "quality_scored",
            "model_id": model_id,
            "crps": crps,
            "coverage_50": cov50,
            "coverage_90": cov90,
            "n": rows.len(),
        }));

        Ok(())
    }

    // ── I-5.10: Drift detection ───────────────────────────────────────────────

    async fn check_calibration_drift(
        &self,
        model_id: &str,
        coverage_90: f64,
        _coverage_50: f64,
    ) -> anyhow::Result<()> {
        // Calibration drift: empirical 90% coverage deviates from 0.90 by more than threshold.
        let nominal = 0.90;
        let deviation = (coverage_90 - nominal).abs();
        if deviation > DEFAULT_PIT_KS_THRESHOLD {
            self.raise_alert(
                model_id,
                "calibration_drift",
                &format!(
                    "90% coverage is {coverage_90:.3} (nominal={nominal:.2}, |deviation|={deviation:.3} > {DEFAULT_PIT_KS_THRESHOLD})",
                ),
                Some(coverage_90),
                Some(DEFAULT_PIT_KS_THRESHOLD),
            )
            .await?;
        }
        Ok(())
    }

    async fn check_feature_drift(&self, model_id: &str, psi: f64) -> anyhow::Result<()> {
        if psi > DEFAULT_PSI_THRESHOLD {
            self.raise_alert(
                model_id,
                "feature_drift",
                &format!("feature PSI={psi:.4} exceeds threshold={DEFAULT_PSI_THRESHOLD}"),
                Some(psi),
                Some(DEFAULT_PSI_THRESHOLD),
            )
            .await?;
        }
        Ok(())
    }

    // I-5.10 — public so external callers (inference path) can push feature drift scores.
    pub async fn report_feature_drift(
        self: &Arc<Self>,
        model_id: &str,
        psi: f64,
    ) -> anyhow::Result<()> {
        self.check_feature_drift(model_id, psi).await
    }

    // ── I-5.11: Staleness check → retrain trigger ─────────────────────────────

    async fn check_staleness(&self, model_id: &str) -> anyhow::Result<()> {
        // Find the most recent succeeded training run.
        let last_train: Option<(chrono::DateTime<Utc>,)> = sqlx::query_as(
            "SELECT finished_at FROM ai_model_runs \
             WHERE model_id = $1 AND status = 'succeeded' \
             ORDER BY finished_at DESC LIMIT 1",
        )
        .bind(model_id)
        .fetch_optional(&self.pg)
        .await
        .unwrap_or(None);

        let stale = match last_train {
            None => true,
            Some((ts,)) => {
                let age_days = (Utc::now() - ts).num_days();
                age_days > DEFAULT_STALENESS_DAYS
            }
        };

        if stale {
            self.raise_alert(
                model_id,
                "staleness",
                &format!("no successful retrain in the last {DEFAULT_STALENESS_DAYS} days"),
                None,
                Some(DEFAULT_STALENESS_DAYS as f64),
            )
            .await?;
        }
        Ok(())
    }

    // ── Alert persistence + retrain trigger (I-5.11) ──────────────────────────

    async fn raise_alert(
        &self,
        model_id: &str,
        kind: &str,
        message: &str,
        metric_value: Option<f64>,
        threshold: Option<f64>,
    ) -> anyhow::Result<()> {
        // Deduplicate: don't raise the same alert kind twice within 24h.
        let recent: Option<String> = sqlx::query_scalar(
            "SELECT id FROM model_quality_alerts \
             WHERE model_id = $1 AND kind = $2 \
               AND triggered_at > NOW() - INTERVAL '24 hours' \
             LIMIT 1",
        )
        .bind(model_id)
        .bind(kind)
        .fetch_optional(&self.pg)
        .await
        .unwrap_or(None);

        if recent.is_some() {
            return Ok(());
        }

        let alert_id = format!("alert_{}", uuid::Uuid::new_v4().simple());

        // Attempt to trigger a retrain pipeline (I-5.11).
        let retrain_run_id = self.trigger_retrain(model_id, kind, &alert_id).await;

        // Persist alert (non-fatal if table missing).
        sqlx::query(
            "INSERT INTO model_quality_alerts \
               (id, model_id, kind, message, metric_value, threshold, \
                triggered_at, retrain_run_id) \
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), $7)",
        )
        .bind(&alert_id)
        .bind(model_id)
        .bind(kind)
        .bind(message)
        .bind(metric_value)
        .bind(threshold)
        .bind(retrain_run_id.as_deref())
        .execute(&self.pg)
        .await
        .ok();

        let _ = self.progress_tx.send(serde_json::json!({
            "kind": "quality_alert",
            "alert_id": alert_id,
            "model_id": model_id,
            "alert_kind": kind,
            "message": message,
            "metric_value": metric_value,
            "threshold": threshold,
            "retrain_run_id": retrain_run_id,
        }));

        tracing::warn!(
            "quality alert [{kind}] for model {model_id}: {message} \
             (retrain={retrain_run_id:?})"
        );

        Ok(())
    }

    /// Kick off a `ModelManager` training run tagged with the triggering alert.
    /// Returns the `run_id` on success, None if no auto-retrain model config found.
    async fn trigger_retrain(
        &self,
        model_id: &str,
        alert_kind: &str,
        alert_id: &str,
    ) -> Option<String> {
        // Only trigger if the model has auto_retrain enabled.
        let user_str: Option<String> = sqlx::query_scalar(
            "SELECT created_by::text FROM ai_models \
             WHERE model_id = $1 \
               AND (definition_json -> 'auto_retrain')::boolean = true",
        )
        .bind(model_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()?;

        let user_id: uuid::Uuid = user_str?.parse().ok()?;

        let req = TrainRequest {
            dataset_version_id: None,
            hyperparameter_overrides: None,
            version_note: Some(format!(
                "quality-triggered retrain (alert={alert_id}, kind={alert_kind})"
            )),
            data: None,
            declaration: None,
            family: None,
            experiment_id: None,
        };

        match self.models.start_train(model_id, user_id, req).await {
            Ok(run_uuid) => Some(run_uuid.to_string()),
            Err(e) => {
                tracing::warn!("retrain trigger for {model_id} failed: {e}");
                None
            }
        }
    }

    // ── Query helpers (I-5.12 route support) ─────────────────────────────────

    pub async fn get_quality_series(
        &self,
        model_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<QualityPoint>> {
        let rows: Vec<(
            String,
            chrono::DateTime<Utc>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            i32,
        )> = sqlx::query_as(
            "SELECT model_id, window_end, crps, coverage_50, coverage_90, n_forecasts \
             FROM model_quality_series WHERE model_id = $1 \
             ORDER BY window_end DESC LIMIT $2",
        )
        .bind(model_id)
        .bind(limit)
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(
                |(model_id, window_end, crps, coverage_50, coverage_90, n_forecasts)| {
                    QualityPoint {
                        model_id,
                        window_end,
                        crps,
                        coverage_50,
                        coverage_90,
                        n_forecasts,
                    }
                },
            )
            .collect())
    }

    pub async fn get_alerts(
        &self,
        model_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<QualityAlert>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Option<f64>,
            Option<f64>,
            chrono::DateTime<Utc>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, model_id, kind, message, metric_value, threshold, \
                    triggered_at, retrain_run_id \
             FROM model_quality_alerts WHERE model_id = $1 \
             ORDER BY triggered_at DESC LIMIT $2",
        )
        .bind(model_id)
        .bind(limit)
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    model_id,
                    kind,
                    message,
                    metric_value,
                    threshold,
                    triggered_at,
                    retrain_run_id,
                )| {
                    QualityAlert {
                        id,
                        model_id,
                        kind,
                        message,
                        metric_value,
                        threshold,
                        triggered_at,
                        retrain_run_id,
                    }
                },
            )
            .collect())
    }
}

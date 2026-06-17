//! Shareable, immutable evaluation reports keyed by (model_id, version, dataset_hash) (I-2.12).

use serde_json::{json, Value};
use sqlx::PgPool;

/// Fetch the full evaluation report for a model version.
///
/// Returns the raw report JSON persisted by `drive_eval`, which includes:
/// CRPS, pinball, log-score, PIT, coverage, reliability, VaR backtests,
/// baselines, DM, overfitting diagnostics, per-fold, and per-regime scores.
pub async fn get_report(
    pg: &PgPool,
    model_id: &str,
    version: i32,
    user_id: uuid::Uuid,
) -> anyhow::Result<Value> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        uuid::Uuid,
        String,
        Option<Value>,
        Option<Value>,
        Option<Value>,
        Option<Value>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT er.eval_id, er.status, er.metrics_json, er.scorecard_json,
                er.regression_report_json, er.report_json,
                er.eval_dataset_version_id::text, er.created_at
         FROM evaluation_runs er
         JOIN ai_models am ON am.model_id = er.model_id
         WHERE er.model_id = $1 AND er.version = $2
           AND am.created_by = $3
           AND er.status = 'succeeded'
         ORDER BY er.created_at DESC LIMIT 1",
    )
    .bind(model_id)
    .bind(version)
    .bind(user_id)
    .fetch_optional(pg)
    .await?;

    let (eval_id, status, metrics, scorecard, regression, report, dataset_ver, created_at) =
        row.ok_or_else(|| anyhow::anyhow!("no completed eval for version {version}"))?;

    // If the sidecar stored a full report_json, return it; otherwise reconstruct
    // from the individual columns for older eval runs.
    if let Some(r) = report.filter(|v| !v.is_null()) {
        return Ok(json!({
            "eval_id": eval_id,
            "model_id": model_id,
            "version": version,
            "status": status,
            "created_at": created_at,
            "dataset_version": dataset_ver,
            "report": r,
            "scorecard": scorecard,
            "regression_report": regression,
        }));
    }

    // Reconstruct from columnar data (back-compat).
    Ok(json!({
        "eval_id": eval_id,
        "model_id": model_id,
        "version": version,
        "status": status,
        "created_at": created_at,
        "dataset_version": dataset_ver,
        "report": {
            "metrics": metrics,
        },
        "scorecard": scorecard,
        "regression_report": regression,
    }))
}

/// Export report as a JSON byte blob for download (shareable format).
pub fn export_report_json(report: &Value) -> anyhow::Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(report)?;
    Ok(bytes)
}

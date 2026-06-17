//! Model leaderboard: rank models and ensembles by CRPS / coverage (I-2.11).

use sqlx::PgPool;
use uuid::Uuid;

/// A single leaderboard row.
#[derive(Debug, serde::Serialize)]
pub struct LeaderboardEntry {
    pub model_id: String,
    pub display_name: String,
    pub model_kind: String,
    pub version: i32,
    pub crps: Option<f64>,
    pub crps_deflated: Option<f64>,
    pub coverage_90: Option<f64>,
    pub pit_calibrated: Option<bool>,
    pub beats_naive: Option<bool>,
    pub scorecard_overall: Option<f64>,
    pub eval_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Query leaderboard for models owned by `user_id`, optionally filtered.
pub async fn query_leaderboard(
    pg: &PgPool,
    user_id: Uuid,
    model_kind: Option<&str>,
    asset_class: Option<&str>,
    metric: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<LeaderboardEntry>> {
    // Each row: latest successful eval per (model_id, version).
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        String,
        i32,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT am.model_id, am.display_name, am.model_kind, er.version,
                er.metrics_json, er.scorecard_json, er.eval_id, er.created_at
         FROM evaluation_runs er
         JOIN ai_models am ON am.model_id = er.model_id
         WHERE am.created_by = $1
           AND er.status = 'succeeded'
           AND ($2::text IS NULL OR am.model_kind = $2)
           AND ($3::text IS NULL OR am.asset_class = $3)
           AND er.created_at = (
               SELECT MAX(er2.created_at)
               FROM evaluation_runs er2
               WHERE er2.model_id = er.model_id
                 AND er2.version = er.version
                 AND er2.status = 'succeeded'
           )
         ORDER BY er.created_at DESC
         LIMIT $4",
    )
    .bind(user_id)
    .bind(model_kind)
    .bind(asset_class)
    .bind(limit)
    .fetch_all(pg)
    .await?;

    let mut entries: Vec<LeaderboardEntry> = rows
        .into_iter()
        .map(
            |(
                model_id,
                display_name,
                model_kind_val,
                version,
                metrics,
                scorecard,
                eval_id,
                created_at,
            )| {
                let m = metrics.as_ref();
                let crps = m
                    .and_then(|v| v.get("crps"))
                    .and_then(serde_json::Value::as_f64);
                let crps_deflated = m
                    .and_then(|v| v.get("crps_deflated"))
                    .and_then(serde_json::Value::as_f64);
                let pit_calibrated = m
                    .and_then(|v| v.get("pit"))
                    .and_then(|v| v.get("calibrated"))
                    .and_then(serde_json::Value::as_bool);
                let beats_naive = m
                    .and_then(|v| v.get("beats_naive"))
                    .and_then(serde_json::Value::as_bool);
                let scorecard_overall = scorecard
                    .as_ref()
                    .and_then(|v| v.get("overall"))
                    .and_then(serde_json::Value::as_f64);

                // Extract 90% interval coverage gap (coverage[3] for typical 7-level grid)
                let coverage_90 = m
                    .and_then(|v| v.get("coverage"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find(|e| {
                                e.get("lower_level")
                                    .and_then(serde_json::Value::as_f64)
                                    .is_some_and(|l| (l - 0.05).abs() < 1e-6)
                            })
                            .and_then(|e| e.get("empirical"))
                            .and_then(serde_json::Value::as_f64)
                    });

                LeaderboardEntry {
                    model_id,
                    display_name,
                    model_kind: model_kind_val,
                    version,
                    crps,
                    crps_deflated,
                    coverage_90,
                    pit_calibrated,
                    beats_naive,
                    scorecard_overall,
                    eval_id,
                    created_at,
                }
            },
        )
        .collect();

    // Sort by chosen metric (lower CRPS = better; higher scorecard = better)
    let sort_metric = metric.unwrap_or("crps");
    match sort_metric {
        "crps" | "crps_deflated" => {
            entries.sort_by(|a, b| {
                let va = if sort_metric == "crps" {
                    a.crps
                } else {
                    a.crps_deflated
                };
                let vb = if sort_metric == "crps" {
                    b.crps
                } else {
                    b.crps_deflated
                };
                // None sorts last; lower is better
                match (va, vb) {
                    (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });
        }
        _ => {
            // Default: sort by scorecard overall desc
            entries.sort_by(|a, b| {
                b.scorecard_overall
                    .partial_cmp(&a.scorecard_overall)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    Ok(entries)
}

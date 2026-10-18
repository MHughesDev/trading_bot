//! `GET /api/agent/data/bar-lag` -- how long after a bar closed it became
//! queryable (S-3.15 `probe_pit_provenance`'s `worst_case_lag_days`).
//!
//! The bars route does not serve `knowledge_time`, so a point-in-time probe could
//! diff two reads but could not say how stale a late arrival was. This reads the
//! same `BarStore::as_of` window under the same project clip and reports the
//! distribution of `knowledge - close`, **over the bars whose knowledge time was
//! actually received**. A bar carrying `BACKFILLED_KNOWLEDGE_TIME` has a
//! knowledge time stamped by the backfill (D-P3), not by a receipt, so its "lag"
//! is a property of the backfill and is counted separately rather than averaged
//! in.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::projects::ProjectStore;
use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct LagQuery {
    pub instrument: String,
    #[serde(default = "default_tf")]
    pub tf: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

fn default_tf() -> String {
    "1h".into()
}

pub async fn bar_lag(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<LagQuery>,
) -> impl IntoResponse {
    use backtest::types::TimeframeExt;
    use dataplane::quality::QualityFlags;

    let store = ProjectStore::new(state.pg.clone());
    let project = match match q.project_id {
        Some(id) => store.get(id).await.and_then(|p| {
            if p.user_id == token.user_id() {
                Ok(p)
            } else {
                Err(crate::projects::ProjectError::NotFound(id.to_string()))
            }
        }),
        None => store.desk(token.user_id()).await,
    } {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "project", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let Some(tf) = <domain::payloads::bar::Timeframe as TimeframeExt>::from_key(&q.tf) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "unknown_timeframe", "timeframe": q.tf})),
        )
            .into_response();
    };
    let (end, cutoff_applied) = project.clip(q.end);
    if q.start >= end {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "empty_window", "cutoff_applied": cutoff_applied})),
        )
            .into_response();
    }
    let bars = match backtest::store::BarStore::connect(&state.clickhouse_url)
        .as_of(end)
        .load_bars(&q.instrument, tf, q.start, end)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let mut lags: Vec<f64> = Vec::new();
    let mut backfilled = 0usize;
    for b in &bars {
        if b.quality_flags
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
        {
            backfilled += 1;
        } else {
            lags.push((b.knowledge_ns - b.ts_ns) as f64 / 1e9);
        }
    }
    lags.sort_by(f64::total_cmp);
    let pick = |p: f64| -> Option<f64> {
        (!lags.is_empty()).then(|| lags[(((lags.len() - 1) as f64) * p).round() as usize])
    };
    Json(json!({
        "instrument": q.instrument,
        "timeframe": q.tf,
        "window": {"start": q.start, "end": end},
        "cutoff_applied": cutoff_applied,
        "bars": bars.len(),
        "backfilled_bars": backfilled,
        "received_bars": lags.len(),
        "lag_seconds": {
            "median": pick(0.5),
            "p95": pick(0.95),
            "max": lags.last().copied(),
            "min": lags.first().copied(),
        },
        "note": "lag is `knowledge - bar close` over bars whose knowledge time was received; backfilled bars carry a stamped knowledge time and are counted, not averaged in",
    }))
    .into_response()
}

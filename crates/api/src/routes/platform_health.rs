//! The platform self-monitoring surface (SPEC §16.2; plan 5.3).
//!
//! One endpoint, every signal. It reports on the platform's own judgment rather
//! than on any model, and it is deliberately the first Phase-5 surface built: a
//! platform that cannot say whether its own gates, exploration floor and feature
//! consistency are behaving cannot tell whether anything else it reports is
//! worth believing.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::{auth::BearerToken, self_monitor, state::AppState};

/// `GET /api/platform/health` — every §16.2 signal for the caller's tenant.
///
/// Signals that cannot be computed are returned as signals in the `unavailable`
/// state, not as an error: one broken query must not hide the other ten. The
/// response is a 200 in that case, and `healthy` is false.
pub async fn health(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    match self_monitor::health(&state.pg, &tenant).await {
        Ok(report) => {
            let alarms: Vec<&str> = report.alarms().iter().map(|s| s.id).collect();
            let unavailable: Vec<&str> = report.unavailable().iter().map(|s| s.id).collect();
            (
                StatusCode::OK,
                Json(json!({
                    "tenant": report.tenant,
                    "generated_at": report.generated_at,
                    "window_days": report.window_days,
                    // `healthy` is false when a signal could not be computed, not
                    // only when one is in alarm. "We do not know" is not "fine".
                    "healthy": report.healthy(),
                    "alarms": alarms,
                    "unavailable": unavailable,
                    "signals": report.signals,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "health_unavailable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// One recorded gate verdict, as the sixteen-gate board reads it.
#[derive(Debug, serde::Serialize)]
pub struct GateVerdictRow {
    pub gate_no: i32,
    pub gate_name: String,
    pub passed: bool,
    pub statistic: Option<f64>,
    pub threshold: Option<f64>,
    pub detail: String,
    pub profile_id: String,
    /// The searching that preceded the verdict (INV-3). Present on every row,
    /// not only the significance gates, because a gate result read without the
    /// trial count behind it is the number §12.2 exists to qualify.
    pub n_eff: Option<f64>,
    pub trial_count_at_eval: Option<i64>,
    pub decided_at: chrono::DateTime<chrono::Utc>,
}

/// `GET /api/platform/gates/{subject}` — the latest stack recorded for one
/// experiment or trial (SPEC §12.3; plan 5.5).
///
/// Returns whatever was recorded, which may be fewer than sixteen rows. The
/// response says how many there are rather than padding to sixteen: a stack that
/// only ran twelve gates is a different fact from one that ran sixteen and
/// passed twelve, and padding would make them look the same.
pub async fn gate_stack(
    State(state): State<AppState>,
    token: BearerToken,
    axum::extract::Path(subject): axum::extract::Path<String>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let mut tx = match ledger::pg::tenant_tx(&state.pg, &tenant).await {
        Ok(tx) => tx,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "gates_unavailable", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    // The newest verdict per gate for this subject. `DISTINCT ON` rather than a
    // window function because the table is append-only: a re-evaluation appends,
    // and the board shows the current answer.
    let rows = sqlx::query_as::<_, (i32, String, bool, Option<f64>, Option<f64>, String, String, Option<f64>, Option<i64>, chrono::DateTime<chrono::Utc>)>(
        "SELECT DISTINCT ON (gate_no)
                gate_no, gate_name, passed, statistic, threshold, detail, profile_id,
                n_eff, trial_count_at_eval, decided_at
         FROM mlops.gate_verdict
         WHERE coalesce(experiment_id, trial_id::text) = $1
         ORDER BY gate_no, decided_at DESC",
    )
    .bind(&subject)
    .fetch_all(&mut *tx)
    .await;
    let _ = tx.commit().await;

    match rows {
        Ok(rows) => {
            let verdicts: Vec<GateVerdictRow> = rows
                .into_iter()
                .map(|(gate_no, gate_name, passed, statistic, threshold, detail, profile_id, n_eff, trial_count_at_eval, decided_at)| {
                    GateVerdictRow {
                        gate_no,
                        gate_name,
                        passed,
                        statistic,
                        threshold,
                        detail,
                        profile_id,
                        n_eff,
                        trial_count_at_eval,
                        decided_at,
                    }
                })
                .collect();
            let recorded = verdicts.len();
            let passed = verdicts.iter().filter(|v| v.passed).count();
            (
                StatusCode::OK,
                Json(json!({
                    "subject": subject,
                    "recorded": recorded,
                    "of": backtest::gates::stack::GATE_COUNT,
                    "passed": passed,
                    // Only a complete, wholly passing stack authorises anything.
                    "complete": recorded == backtest::gates::stack::GATE_COUNT,
                    "blocked_at": verdicts.iter().find(|v| !v.passed).map(|v| v.gate_no),
                    "profile_id": verdicts.first().map(|v| v.profile_id.clone()),
                    "verdicts": verdicts,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "gates_unavailable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// One row of the agent action timeline (SPEC §15; plan 5.6).
#[derive(Debug, serde::Serialize)]
pub struct TimelineRow {
    /// `audit` or `decision` — the two things a reviewer needs side by side.
    pub kind: &'static str,
    pub at: chrono::DateTime<chrono::Utc>,
    /// The action, or the decision kind.
    pub what: String,
    /// `pre` / `post` for an audit row; the tier for a decision row.
    pub phase: String,
    /// The verdict for an audit row.
    pub verdict: Option<String>,
    pub envelope: Option<String>,
    pub actor_kind: String,
    pub actor_id: String,
    pub on_behalf_of: Option<String>,
    /// The candidate set a decision chose from, and with what probability.
    pub propensity: Option<f64>,
    pub exploration_flag: Option<bool>,
    pub candidate_count: Option<i64>,
    pub detail: serde_json::Value,
}

/// `GET /api/platform/timeline` — the agent action review timeline (plan 5.6).
///
/// Audit rows and decision rows interleaved in time, because reviewing an agent
/// session means asking two questions at once: what did it try to do, and what
/// did it choose from. Separately they are two lists nobody correlates.
///
/// The `pre` rows are the reason this view is worth having. A denied action
/// leaves no other trace — without them, "the agent never tried that" and "the
/// agent tried and was stopped" look identical afterwards (ADR-P2-21).
pub async fn timeline(
    State(state): State<AppState>,
    token: BearerToken,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let limit: i64 = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(200).clamp(1, 1000);

    let mut tx = match ledger::pg::tenant_tx(&state.pg, &tenant).await {
        Ok(tx) => tx,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "timeline_unavailable", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    let audits = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, String, String, String, String, String, String, Option<String>, serde_json::Value)>(
        "SELECT occurred_at, action, record_phase, verdict, envelope, actor_kind, actor_id,
                on_behalf_of, request
         FROM mlops.audit_event
         WHERE tenant_id = $1
         ORDER BY occurred_at DESC
         LIMIT $2",
    )
    .bind(&tenant)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await;

    let decisions = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, String, String, f64, bool, i64, String, String)>(
        "SELECT decided_at, kind, decision_tier, propensity, exploration_flag,
                coalesce(jsonb_array_length(candidate_set), 0)::bigint,
                actor_kind, actor_id
         FROM mlops.decision
         WHERE tenant_id = $1
         ORDER BY decided_at DESC
         LIMIT $2",
    )
    .bind(&tenant)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await;
    let _ = tx.commit().await;

    let (Ok(audits), Ok(decisions)) = (audits, decisions) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "timeline_unavailable" })),
        )
            .into_response();
    };

    let mut rows: Vec<TimelineRow> = Vec::with_capacity(audits.len() + decisions.len());
    for (at, action, phase, verdict, envelope, actor_kind, actor_id, on_behalf_of, request) in audits {
        rows.push(TimelineRow {
            kind: "audit",
            at,
            what: action,
            phase,
            verdict: Some(verdict),
            envelope: Some(envelope),
            actor_kind,
            actor_id,
            on_behalf_of,
            propensity: None,
            exploration_flag: None,
            candidate_count: None,
            detail: request,
        });
    }
    for (at, kind, tier, propensity, exploration, candidates, actor_kind, actor_id) in decisions {
        rows.push(TimelineRow {
            kind: "decision",
            at,
            what: kind,
            phase: tier,
            verdict: None,
            envelope: None,
            actor_kind,
            actor_id,
            on_behalf_of: None,
            propensity: Some(propensity),
            exploration_flag: Some(exploration),
            candidate_count: Some(candidates),
            detail: json!({}),
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.at));
    rows.truncate(usize::try_from(limit).unwrap_or(200));

    // A `pre` with no matching `post` is an action that was stopped. Counted
    // here so a reviewer sees it without reading every row.
    let prevented = rows
        .iter()
        .filter(|r| r.kind == "audit" && r.verdict.as_deref().is_some_and(|v| v == "denied" || v == "pending_approval"))
        .count();

    (
        StatusCode::OK,
        Json(json!({
            "rows": rows,
            "prevented": prevented,
            "limit": limit,
        })),
    )
        .into_response()
}

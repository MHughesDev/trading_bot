//! The operator's alert inbox (FB-RX-005).
//!
//! One list of things that need a person, from three sources that already exist:
//!
//!   * `monitor.events` (COMP-007) — breaches, scale-downs, halts, kill-switch
//!     trips, maintenance windows;
//!   * `price_alerts` that have fired (`triggered_at IS NOT NULL`);
//!   * `calibration` — a forecaster whose interval coverage left tolerance in two
//!     consecutive judged windows (`research::refit::coverage_drift` over
//!     `model_quality_series`; FEAT-003 Phase 4, Set R-b RB-5.8). Migration 0245 widens
//!     the `alert_inbox_state.source` CHECK to admit it.
//!
//! The desk's approvals queue (`/desk/approvals`) is **not** here: that answers a
//! running agent's pause and has its own surface, and a second list of the same
//! decisions would be two places to answer one question.
//!
//! Routes (registered together by the orchestrator):
//!
//!   * `GET  /api/alert-inbox` — the items and the unread count;
//!   * `POST /api/alert-inbox/{source}/{id}/acknowledge`
//!   * `POST /api/alert-inbox/{source}/{id}/dismiss`
//!
//! # Where the facts live
//!
//! Acknowledge and dismiss are **server state**, in `alert_inbox_state` (keyed by
//! user, source and item), not in the browser: an alert dismissed on one device
//! must not return on another. The table is a migration (see the report that
//! accompanies this file) and every statement here is the exact set its
//! `app_role` grant allows: SELECT, INSERT, UPDATE.
//!
//! # Severity is defined once, here
//!
//! [`monitor_severity`] is the only mapping from a monitor event kind to a
//! severity. The client renders the word it is sent and has no table of its own.
//!
//! # A trip cannot be dismissed while it is in force
//!
//! A `tripped` event whose scope is **currently halted** in `monitor.kill_switch`
//! is `critical` and `dismissible: false`, and `dismiss` refuses it with 409
//! `pinned`. It can be acknowledged — that records that a person has seen it —
//! but it stays in the inbox until the switch is reset, at which point it
//! becomes an ordinary item. The check is made at the moment of the dismiss,
//! against the durable table, not against what the client last rendered.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// The only place a monitor event kind becomes a severity.
#[must_use]
pub fn monitor_severity(kind: &str) -> &'static str {
    match kind {
        "tripped" | "halted" => "critical",
        "breach" | "scaled_down" | "warning" | "window_opened" => "warning",
        _ => "info",
    }
}

/// The monitor event kinds that belong in an inbox at all. A blip is recorded
/// and acted on by nothing (COMP-007 §4.1); an inbox that listed them would be
/// one nobody reads.
const INBOX_KINDS: &[&str] = &[
    "tripped",
    "halted",
    "breach",
    "scaled_down",
    "warning",
    "window_opened",
];

/// Price alerts that fired are worth a look, not a page.
const PRICE_SEVERITY: &str = "info";

/// A calibration drift is a model whose stated uncertainty is wrong, which is a reason to
/// stop trusting its intervals: a warning, never critical (it does not halt anything).
const CALIBRATION_SEVERITY: &str = "warning";

/// How many of a model's most recent quality windows the drift rule reads.
const CALIBRATION_WINDOWS: i64 = 12;

/// One calibration drift, ready to be an inbox item.
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationDrift {
    /// Deterministic in the model and the window that completed the pair, so a person's
    /// acknowledge or dismiss refers to the same item on every request.
    pub id: Uuid,
    pub model_id: String,
    pub at: DateTime<Utc>,
    pub message: String,
}

/// The stable id of a drift: a hash of the model and the end of the second bad window.
#[must_use]
pub fn calibration_item_id(model_id: &str, window_end: DateTime<Utc>) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"calibration\x1f");
    h.update(model_id.as_bytes());
    h.update(b"\x1f");
    h.update(window_end.timestamp().to_be_bytes());
    let d = h.finalize();
    Uuid::from_slice(&d[..16]).expect("sixteen bytes")
}

/// The drift, if any, in one model's windows (oldest first). Pure: the database read is
/// [`calibration_drifts`]. The rule is `research::refit::coverage_drift` and is not
/// restated here.
#[must_use]
pub fn drift_of(
    model_id: &str,
    windows: &[(DateTime<Utc>, research::refit::CoverageWindow)],
) -> Option<CalibrationDrift> {
    let series: Vec<research::refit::CoverageWindow> = windows.iter().map(|(_, w)| *w).collect();
    let d = research::refit::coverage_drift(&series, &research::refit::DriftTolerance::default())?;
    let at = windows.get(d.at_window)?.0;
    Some(CalibrationDrift {
        id: calibration_item_id(model_id, at),
        model_id: model_id.to_string(),
        at,
        message: d.message,
    })
}

/// Every model whose recent quality windows show a calibration drift.
///
/// # Errors
/// The database. A missing `model_quality_series` is an error and not an empty list, so a
/// deployment that never applied 0222 is visible rather than quiet.
pub async fn calibration_drifts(pg: &sqlx::PgPool) -> Result<Vec<CalibrationDrift>, sqlx::Error> {
    let rows: Vec<(String, DateTime<Utc>, Option<f64>, Option<f64>, i32)> = sqlx::query_as(
        "SELECT model_id, window_end, coverage_50, coverage_90, n_forecasts FROM (
             SELECT model_id, window_end, coverage_50, coverage_90, n_forecasts,
                    row_number() OVER (PARTITION BY model_id ORDER BY window_end DESC) AS rn
               FROM model_quality_series
              WHERE coverage_50 IS NOT NULL AND coverage_90 IS NOT NULL
         ) t WHERE rn <= $1 ORDER BY model_id, window_end",
    )
    .bind(CALIBRATION_WINDOWS)
    .fetch_all(pg)
    .await?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let model = rows[i].0.clone();
        let mut windows = Vec::new();
        while i < rows.len() && rows[i].0 == model {
            let (_, end, c50, c90, n) = &rows[i];
            windows.push((
                *end,
                research::refit::CoverageWindow {
                    coverage_50: c50.unwrap_or(f64::NAN),
                    coverage_90: c90.unwrap_or(f64::NAN),
                    n: u32::try_from(*n).unwrap_or(0),
                },
            ));
            i += 1;
        }
        if let Some(d) = drift_of(&model, &windows) {
            out.push(d);
        }
    }
    Ok(out)
}

fn server_error(detail: impl std::fmt::Display) -> Response {
    tracing::error!(error = %detail, "alert inbox failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "inbox_unavailable" })),
    )
        .into_response()
}

fn refuse(status: StatusCode, error: &str, detail: &str) -> Response {
    (status, Json(json!({ "error": error, "detail": detail }))).into_response()
}

/// Whether a `tripped` event is still in force, given the halted scopes.
#[must_use]
pub fn is_pinned(kind: &str, scope: &str, halted: &HashSet<String>) -> bool {
    kind == "tripped" && halted.contains(scope)
}

async fn halted_scopes(state: &AppState) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT scope FROM monitor.kill_switch WHERE state = 'halted'")
            .fetch_all(&state.pg)
            .await?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

type MonitorRow = (
    Uuid,
    DateTime<Utc>,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// `GET /api/alert-inbox`
pub async fn inbox(
    token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<Value>, Response> {
    let user = token.user_id();
    let halted = halted_scopes(&state).await.map_err(server_error)?;

    let acted: HashMap<(String, String), String> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT source, item_id, state FROM alert_inbox_state WHERE user_id = $1",
    )
    .bind(user)
    .fetch_all(&state.pg)
    .await
    .map_err(server_error)?
    .into_iter()
    .map(|(source, id, st)| ((source, id), st))
    .collect();

    let events = sqlx::query_as::<_, MonitorRow>(
        "SELECT event_id, occurred_at, scope, kind, metric, reason
           FROM monitor.events
          WHERE kind = ANY($1)
            AND NOT is_drill
            AND occurred_at >= now() - interval '30 days'
          ORDER BY occurred_at DESC
          LIMIT 200",
    )
    .bind(
        INBOX_KINDS
            .iter()
            .map(|k| (*k).to_owned())
            .collect::<Vec<_>>(),
    )
    .fetch_all(&state.pg)
    .await
    .map_err(server_error)?;

    let prices =
        sqlx::query_as::<_, (Uuid, String, String, String, DateTime<Utc>, Option<String>)>(
            "SELECT alert_id, instrument_id, kind, value::text, triggered_at, triggered_price::text
           FROM price_alerts
          WHERE user_id = $1 AND triggered_at IS NOT NULL
          ORDER BY triggered_at DESC
          LIMIT 100",
        )
        .bind(user)
        .fetch_all(&state.pg)
        .await
        .map_err(server_error)?;

    let drifts = calibration_drifts(&state.pg).await.map_err(server_error)?;

    let mut items: Vec<(DateTime<Utc>, Value, bool)> = Vec::new();

    for d in drifts {
        let id = d.id.to_string();
        let state_word = acted
            .get(&("calibration".to_owned(), id.clone()))
            .map_or("unread", String::as_str);
        if state_word == "dismissed" {
            continue;
        }
        items.push((
            d.at,
            json!({
                "source": "calibration",
                "id": id,
                "at": d.at,
                "severity": CALIBRATION_SEVERITY,
                "state": state_word,
                "title": format!("calibration drift — {}", d.model_id),
                "detail": d.message,
                "dismissible": true,
                "pinned_reason": Value::Null,
            }),
            state_word == "unread",
        ));
    }

    for (event_id, at, scope, kind, metric, reason) in events {
        let id = event_id.to_string();
        let state_word = acted
            .get(&("monitor".to_owned(), id.clone()))
            .map_or("unread", String::as_str);
        if state_word == "dismissed" {
            continue;
        }
        let pinned = is_pinned(&kind, &scope, &halted);
        let detail = [metric.map(|m| format!("metric {m}")), reason]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        items.push((
            at,
            json!({
                "source": "monitor",
                "id": id,
                "at": at,
                "severity": monitor_severity(&kind),
                "state": state_word,
                "title": format!("{} — {}", kind.replace('_', " "), scope),
                "detail": if detail.is_empty() { Value::Null } else { Value::String(detail) },
                "dismissible": !pinned,
                "pinned_reason": if pinned {
                    json!("the kill switch is still tripped for this scope; reset it to clear this")
                } else {
                    Value::Null
                },
            }),
            state_word == "unread",
        ));
    }

    for (alert_id, instrument, kind, value, at, price) in prices {
        let id = alert_id.to_string();
        let state_word = acted
            .get(&("price".to_owned(), id.clone()))
            .map_or("unread", String::as_str);
        if state_word == "dismissed" {
            continue;
        }
        items.push((
            at,
            json!({
                "source": "price",
                "id": id,
                "at": at,
                "severity": PRICE_SEVERITY,
                "state": state_word,
                "title": format!("{instrument} {} {value}", kind.replace('_', " ")),
                "detail": price.map(|p| format!("fired at {p}")),
                "dismissible": true,
                "pinned_reason": Value::Null,
            }),
            state_word == "unread",
        ));
    }

    // Newest first across both sources.
    items.sort_by_key(|item| std::cmp::Reverse(item.0));
    let unread = items.iter().filter(|(_, _, u)| *u).count();
    Ok(Json(json!({
        "unread": unread,
        "items": items.into_iter().map(|(_, v, _)| v).collect::<Vec<_>>(),
    })))
}

fn valid_source(source: &str) -> bool {
    matches!(source, "monitor" | "price" | "calibration")
}

/// `POST /api/alert-inbox/{source}/{id}/acknowledge`
pub async fn acknowledge(
    token: BearerToken,
    State(state): State<AppState>,
    Path((source, id)): Path<(String, String)>,
) -> Result<Json<Value>, Response> {
    if !valid_source(&source) || id.parse::<Uuid>().is_err() {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "invalid_item",
            "unknown source or id",
        ));
    }
    // A dismissed item never comes back as merely acknowledged.
    sqlx::query(
        "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
         VALUES ($1, $2, $3, 'acknowledged')
         ON CONFLICT (user_id, source, item_id)
         DO UPDATE SET state = 'acknowledged', acted_at = now()
         WHERE alert_inbox_state.state <> 'dismissed'",
    )
    .bind(token.user_id())
    .bind(&source)
    .bind(&id)
    .execute(&state.pg)
    .await
    .map_err(server_error)?;
    Ok(Json(
        json!({ "source": source, "id": id, "state": "acknowledged" }),
    ))
}

/// `POST /api/alert-inbox/{source}/{id}/dismiss`
pub async fn dismiss(
    token: BearerToken,
    State(state): State<AppState>,
    Path((source, id)): Path<(String, String)>,
) -> Result<Json<Value>, Response> {
    let Ok(uuid) = id.parse::<Uuid>() else {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "invalid_item",
            "unknown source or id",
        ));
    };
    if !valid_source(&source) {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "invalid_item",
            "unknown source or id",
        ));
    }
    if source == "monitor" {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT kind, scope FROM monitor.events WHERE event_id = $1")
                .bind(uuid)
                .fetch_optional(&state.pg)
                .await
                .map_err(server_error)?;
        let Some((kind, scope)) = row else {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "not_found",
                "no such monitor event",
            ));
        };
        let halted = halted_scopes(&state).await.map_err(server_error)?;
        if is_pinned(&kind, &scope, &halted) {
            return Err(refuse(
                StatusCode::CONFLICT,
                "pinned",
                "the kill switch is still tripped for this scope; a trip cannot be dismissed while it is in force",
            ));
        }
    } else if source == "calibration" {
        // The id is derived, so it is checked against what is drifting now: a made-up id
        // would otherwise be a row nobody can ever see.
        let known = calibration_drifts(&state.pg).await.map_err(server_error)?;
        if !known.iter().any(|d| d.id == uuid) {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "not_found",
                "no such calibration alert",
            ));
        }
    } else {
        let own: Option<(Uuid,)> = sqlx::query_as(
            "SELECT alert_id FROM price_alerts WHERE alert_id = $1 AND user_id = $2",
        )
        .bind(uuid)
        .bind(token.user_id())
        .fetch_optional(&state.pg)
        .await
        .map_err(server_error)?;
        if own.is_none() {
            return Err(refuse(StatusCode::NOT_FOUND, "not_found", "no such alert"));
        }
    }
    sqlx::query(
        "INSERT INTO alert_inbox_state (user_id, source, item_id, state)
         VALUES ($1, $2, $3, 'dismissed')
         ON CONFLICT (user_id, source, item_id)
         DO UPDATE SET state = 'dismissed', acted_at = now()",
    )
    .bind(token.user_id())
    .bind(&source)
    .bind(&id)
    .execute(&state.pg)
    .await
    .map_err(server_error)?;
    Ok(Json(
        json!({ "source": source, "id": id, "state": "dismissed" }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trip_is_critical_and_a_breach_is_a_warning() {
        assert_eq!(monitor_severity("tripped"), "critical");
        assert_eq!(monitor_severity("halted"), "critical");
        assert_eq!(monitor_severity("breach"), "warning");
        assert_eq!(monitor_severity("scale_released"), "info");
    }

    #[test]
    fn every_inbox_kind_has_a_deliberate_severity() {
        // A kind added to the inbox without a decision about its severity would
        // silently be `info`; this makes that a failing test instead.
        for k in INBOX_KINDS {
            assert_ne!(
                monitor_severity(k),
                "info",
                "{k} is in the inbox but ranks as info"
            );
        }
    }

    #[test]
    fn a_trip_is_pinned_only_while_its_scope_is_halted() {
        let halted: HashSet<String> = ["global".to_owned()].into_iter().collect();
        assert!(is_pinned("tripped", "global", &halted));
        assert!(
            !is_pinned("tripped", "strategy:s1", &halted),
            "another scope is not this trip"
        );
        assert!(
            !is_pinned("breach", "global", &halted),
            "only a trip is pinned"
        );
        assert!(
            !is_pinned("tripped", "global", &HashSet::new()),
            "after the reset it is ordinary"
        );
    }

    fn win(
        day: i64,
        c50: f64,
        c90: f64,
        n: u32,
    ) -> (DateTime<Utc>, research::refit::CoverageWindow) {
        (
            DateTime::from_timestamp(1_700_000_000 + day * 86_400, 0).unwrap(),
            research::refit::CoverageWindow {
                coverage_50: c50,
                coverage_90: c90,
                n,
            },
        )
    }

    #[test]
    fn two_consecutive_bad_windows_raise_one_calibration_item_with_a_stable_id() {
        let good = |d| win(d, 0.5, 0.9, 100);
        let bad = |d| win(d, 0.8, 0.99, 100);
        let ok = vec![good(0), bad(1), good(2), bad(3), good(4)];
        assert!(
            drift_of("m", &ok).is_none(),
            "isolated bad windows are not a drift"
        );
        let drifting = vec![good(0), bad(1), bad(2), bad(3)];
        let d = drift_of("m", &drifting).expect("two in a row");
        assert_eq!(
            d.at, drifting[2].0,
            "dated at the window that completed the pair"
        );
        assert_eq!(
            d.id,
            drift_of("m", &drifting).unwrap().id,
            "the same drift has the same id"
        );
        assert_ne!(
            d.id,
            calibration_item_id("other", d.at),
            "and the id is per model"
        );
        let mut longer = drifting.clone();
        longer.push(bad(4));
        assert_eq!(
            drift_of("m", &longer).unwrap().id,
            d.id,
            "a longer run is not a second alert"
        );
    }

    #[test]
    fn thin_windows_neither_raise_nor_hide_a_drift() {
        let thin = vec![
            win(0, 0.99, 0.5, 3),
            win(1, 0.99, 0.5, 3),
            win(2, 0.5, 0.9, 100),
        ];
        assert!(
            drift_of("m", &thin).is_none(),
            "a window under min_n judges nothing"
        );
    }

    #[test]
    fn calibration_is_an_accepted_source_and_is_checked_against_what_is_drifting() {
        assert!(valid_source("calibration") && valid_source("monitor") && valid_source("price"));
        assert!(!valid_source("anything_else"));
        let src = include_str!("alerts_inbox.rs");
        let body = &src[src.find("pub async fn dismiss(").unwrap()..];
        assert!(
            body.contains("calibration_drifts(&state.pg)"),
            "dismissing a derived id verifies it"
        );
    }

    #[test]
    fn a_blip_never_reaches_the_inbox() {
        assert!(!INBOX_KINDS.contains(&"blip"));
        assert!(!INBOX_KINDS.contains(&"drill"));
    }

    #[test]
    fn the_dismiss_check_reads_the_durable_table_not_the_client() {
        let src = include_str!("alerts_inbox.rs");
        let at = src.find("pub async fn dismiss(").unwrap();
        let body = &src[at..];
        assert!(body.contains("halted_scopes(&state)"));
        assert!(
            body.find("is_pinned(").unwrap() < body.find("INSERT INTO alert_inbox_state").unwrap()
        );
    }
}

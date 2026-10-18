//! Data API v2 (DATA-005 §4, §5; DA-01…DA-09, DA-15).
//!
//! This is where the research cutoff is enforced. Every read resolves the caller's
//! project, clips the window to that project's horizon, and records what it did in
//! the response manifest. The agent cannot bypass it, because there is no other way
//! to reach bars — and that, not any instruction in a prompt, is what makes the
//! holdout mean something (D-10, ADR-0025).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use storage::clickhouse::forward;

use crate::projects::{Project, ProjectError};
use crate::{auth::BearerToken, state::AppState};

fn map_project_err(e: ProjectError) -> axum::response::Response {
    let (status, code) = match &e {
        ProjectError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        ProjectError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
        ProjectError::LiveDataDeskOnly(_) => (StatusCode::FORBIDDEN, "live_data_desk_only"),
        ProjectError::DeskExploratoryOnly(_) => (StatusCode::FORBIDDEN, "desk_exploratory_only"),
        ProjectError::Sqlx(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let mut body = json!({ "error": code, "message": e.to_string() });
    // A refusal that does not say what to do instead just invites a retry.
    if matches!(e, ProjectError::LiveDataDeskOnly(_)) {
        body["fix"] = json!("re-issue this request against your Desk project");
    }
    (status, Json(body)).into_response()
}

/// Resolves the project a data request is made in.
///
/// Defaults to the caller's Desk when no project is named. That default is
/// deliberately the *restrictive* one for research purposes: the Desk can see live
/// data but cannot run confirmatory work, so an unscoped read can never quietly
/// consume a research project's holdout.
async fn resolve_project(
    state: &AppState,
    user_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Project, ProjectError> {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    match project_id {
        Some(id) => {
            let project = store.get(id).await?;
            if project.user_id != user_id {
                // Reported as missing rather than forbidden: a caller should not be
                // able to discover which project ids exist.
                return Err(ProjectError::NotFound(id.to_string()));
            }
            Ok(project)
        }
        None => store.desk(user_id).await,
    }
}

#[derive(Debug, Deserialize)]
pub struct BarsQuery {
    pub instrument: String,
    #[serde(default = "default_timeframe")]
    pub tf: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// `json` (the default) or `parquet` (a `parquet_extract` artifact handle).
    #[serde(default)]
    pub format: Option<String>,
}

fn default_timeframe() -> String {
    "1m".into()
}

/// GET /api/data/bars — point-in-time bars, clipped to the project's cutoff.
pub async fn get_bars(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<BarsQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };

    // The three clips that matter, in order. `as_of` and `end` are separate concepts —
    // `end` bounds which bars, `as_of` bounds which *revisions* of them — and both
    // must respect the horizon or a late correction could leak the future.
    let (effective_end, cutoff_applied) = project.clip(q.end);
    let requested_as_of = q.as_of.unwrap_or(effective_end);
    let (effective_as_of, _) = project.clip(requested_as_of);

    if q.start >= effective_end {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_window",
                "message": format!(
                    "start {} is at or after the effective end {}",
                    q.start, effective_end
                ),
                "fix": if cutoff_applied.is_some() {
                    "the window was clipped to this project's research cutoff; \
                     request an earlier window, or use the Desk for recent data"
                } else {
                    "request a window with start < end"
                },
                "cutoff_applied": cutoff_applied,
            })),
        )
            .into_response();
    }

    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(effective_as_of);
    let timeframe =
        match <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(&q.tf) {
            Some(tf) => tf,
            None => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "unknown_timeframe",
                        "message": format!("unknown timeframe {:?}", q.tf),
                        "known": ["1s", "1m", "5m", "15m", "1h", "4h", "1d"],
                    })),
                )
                    .into_response()
            }
        };

    let bars = match store
        .load_bars(&q.instrument, timeframe, q.start, effective_end)
        .await
    {
        Ok(bars) => bars,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    let limit = q.limit.unwrap_or(5_000).clamp(1, 50_000) as usize;
    let truncated = bars.len() > limit;
    let rows: Vec<serde_json::Value> = bars
        .iter()
        .take(limit)
        .map(|b| {
            json!({
                "ts_ns": b.ts_ns,
                "open": b.open.to_string(),
                "high": b.high.to_string(),
                "low": b.low.to_string(),
                "close": b.close.to_string(),
                "volume": b.volume.to_string(),
                "trade_count": b.trade_count,
            })
        })
        .collect();

    // Every read is exploration, and exploration is logged rather than counted
    // (D-13, JB-11). The variables are recorded so the hypothesis registry can tell
    // later whether a hypothesis was formed before or after looking.
    if let Some(jobs) = state.jobs.as_ref() {
        let _ = jobs
            .log_exploration(jobs::ExplorationEntry {
                project_id: project.project_id,
                user_id: token.user_id(),
                session_id: None,
                source: if project.is_desk() {
                    "desk"
                } else {
                    "data_api"
                },
                instruments: vec![q.instrument.clone()],
                timeframe: Some(q.tf.clone()),
                window_start: Some(q.start),
                window_end: Some(effective_end),
                variables: vec![
                    "open".into(),
                    "high".into(),
                    "low".into(),
                    "close".into(),
                    "volume".into(),
                ],
                description: format!(
                    "bars {} {} [{}, {})",
                    q.instrument, q.tf, q.start, effective_end
                ),
                handle: None,
            })
            .await;
    }

    // The grade travels with the data (DATA-005 §6; P-2.2): the newest persisted
    // `data_qc_reports` row for this series, with its id and reasons. A series that
    // has never been graded says "ungraded" -- never a letter it did not earn.
    let qc = match storage::clickhouse::qc_reports::latest_grade(&state.pg, &q.instrument, &q.tf)
        .await
    {
        Ok(Some(g)) => json!({
            "grade": g.grade,
            "report_id": g.report_id,
            "venue": g.venue_id,
            "reasons": g.reasons,
            "config_version": g.config_version,
            "graded_at": g.graded_at,
        }),
        Ok(None) => json!({"grade": "ungraded", "report_id": null}),
        Err(e) => json!({"grade": "unknown", "report_id": null, "error": e.to_string()}),
    };

    // Set L Phase 2 (LEGACY): the manifest gains `data_snapshot_id`, and
    // `?format=parquet` stores the rows as a `parquet_extract` artifact.
    let manifest = json!({
        "type": "bars",
        "instrument": q.instrument,
        "timeframe": q.tf,
        "window": {"start": q.start, "end": effective_end},
        "as_of": effective_as_of,
        // Stated, not hidden: the caller asked for more than this project may
        // see, and the manifest says exactly where the line was drawn.
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
        "truncated": truncated,
        "qc_grade": qc["grade"].clone(),
        "qc_report_id": qc["report_id"].clone(),
        "qc": qc,
    });
    deliver(
        &state,
        &project,
        "bars",
        json!({
            "instrument": q.instrument, "tf": q.tf, "start": q.start,
            "end": effective_end, "limit": limit,
        }),
        effective_as_of,
        manifest,
        "bars",
        rows,
        BAR_COLS,
        q.format.as_deref(),
    )
    .await
}

const BAR_COLS: &[(&str, PqCol)] = &[
    ("ts_ns", PqCol::TsNs),
    ("open", PqCol::Dec),
    ("high", PqCol::Dec),
    ("low", PqCol::Dec),
    ("close", PqCol::Dec),
    ("volume", PqCol::Dec),
    ("trade_count", PqCol::I64),
];

/// Query for [`get_funding`].
#[derive(Debug, Deserialize)]
pub struct FundingQuery {
    pub venue: String,
    pub instrument: String,
    /// Inclusive lower bound on the settlement instant.
    pub from: DateTime<Utc>,
    /// Exclusive upper bound on the settlement instant.
    pub to: DateTime<Utc>,
    /// Rows whose `knowledge_time` is later are invisible. Defaults to the
    /// effective `to`, and is clipped to the project horizon either way -- a
    /// restated funding rate is a late correction, and a late correction read
    /// without a horizon is a leak.
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    /// `realized` (the default) or `predicted`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/catalog — instruments with coverage, as the project may see them.
pub async fn get_catalog(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };

    // Coverage is computed through the same horizon as the data itself. A catalogue
    // that advertised bars the project cannot read would be a slow leak of the
    // holdout: "history ends on the 4th" is information about the future.
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(project.horizon());
    match store.list_coverage().await {
        Ok(coverage) => (
            StatusCode::OK,
            Json(json!({
                "project_id": project.project_id,
                "project_kind": project.kind.as_str(),
                "horizon": project.horizon(),
                "instruments": coverage.iter().map(|c| json!({
                    "instrument_id": c.instrument_id,
                    "timeframe": c.timeframe,
                    "bars": c.bars,
                    "first_ns": c.first_ns,
                    "last_ns": c.last_ns,
                })).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "query_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/data/live/{instrument} — Desk only (DA-15).
pub async fn get_live(
    State(state): State<AppState>,
    token: BearerToken,
    Path(instrument): Path<String>,
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    if let Err(e) = project.allow_live() {
        return map_project_err(e);
    }

    let store = backtest::store::BarStore::connect(&state.clickhouse_url);
    let timeframe = domain::payloads::bar::Timeframe::Minutes1;
    match store.last_bar_time(&instrument, timeframe).await {
        Ok(Some(last)) => {
            let age_s = (Utc::now() - last).num_seconds();
            (
                StatusCode::OK,
                Json(json!({
                    "instrument": instrument,
                    "last_bar_time": last,
                    "last_bar_age_s": age_s,
                    "project_id": project.project_id,
                })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no_data", "instrument": instrument})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "query_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

// ── projects ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub instruments: Vec<String>,
    #[serde(default)]
    pub research_cutoff: Option<DateTime<Utc>>,
    #[serde(default)]
    pub holdout_days: Option<i64>,
}

/// POST /api/projects — create a research project.
pub async fn create_project(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    match store
        .create_research(
            token.user_id(),
            &req.name,
            req.goal.as_deref(),
            &req.instruments,
            req.research_cutoff,
            req.holdout_days,
        )
        .await
    {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(e) => map_project_err(e),
    }
}

/// GET /api/projects — the caller's projects, Desk included (created on demand).
pub async fn list_projects(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    // Touch the Desk so it exists from the first listing rather than appearing later.
    let _ = store.desk(token.user_id()).await;
    match store.list(token.user_id()).await {
        Ok(list) => (StatusCode::OK, Json(json!({"projects": list}))).into_response(),
        Err(e) => map_project_err(e),
    }
}

/// GET /api/projects/{id}
pub async fn get_project(
    State(state): State<AppState>,
    token: BearerToken,
    Path(project_id): Path<Uuid>,
) -> impl IntoResponse {
    match resolve_project(&state, token.user_id(), Some(project_id)).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => map_project_err(e),
    }
}

// ── funding (Set S S-3.14, S-3.15; DATA-005 §11; D-P3) ────────────────────────

/// `GET /api/data/funding` — the settled or predicted funding series for one
/// perpetual, as the caller's project may see it.
///
/// **This route exists because three readers had no caller.** `read_funding`,
/// `read_open_interest` and `funding_at` landed in
/// `storage::clickhouse::forward` on 2026-09-22 as passengers on a commit about
/// agent pause/resume, and had zero callers anywhere outside their own file: no
/// MCP tool, no route, no execution-side consumer. `apply_funding_costs`
/// (S-3.14) and `get_funding_history` (S-3.15) both need this door, so it is one
/// route against two tasks.
///
/// ## Three outcomes, and why none of them is an empty array
///
/// A perpetual's carry is the one quantity where a missing measurement read as
/// zero is *profitable-looking*: funding is a cost, so treating an unknown
/// interval as free biases every short-carry strategy upward. The reader already
/// draws the distinction with types (`SeriesRead::NoSuchSeries` against
/// `Present { rows: [] }`), and this route's whole job is to carry it across the
/// wire without flattening it:
///
/// * **the series is absent from this database** — `funding` is JSON `null`, with
///   `unmeasured` naming the table and the database. A caller writing
///   `data.funding.length` throws instead of reading `0`, and `?? []` becomes a
///   decision somebody made.
/// * **the series exists and the window is empty** — `funding` is `[]` and
///   `measured` is `true`. That *is* a measurement: nothing settled in that
///   window.
/// * **rows** — each one carrying its own `knowledge_time` and whether it is
///   admissible as forward evidence, because a backfilled knowledge time is not
///   a receipt (D-P3) and Gate 15 may not take it.
pub async fn get_funding(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<FundingQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };

    let kind = match parse_funding_kind(q.kind.as_deref()) {
        Ok(k) => k,
        Err(other) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "unknown_funding_kind",
                    "message": format!("unknown funding kind {other:?}"),
                    "known": ["realized", "predicted"],
                })),
            )
                .into_response()
        }
    };

    // The same two clips as `get_bars`, and for the same reason: `to` bounds which
    // settlements, `as_of` bounds which *revisions* of them, and a venue does
    // restate funding.
    let (effective_to, cutoff_applied) = project.clip(q.to);
    let (effective_as_of, _) = project.clip(q.as_of.unwrap_or(effective_to));
    if q.from >= effective_to {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_window",
                "message": format!("from {} is at or after the effective to {}", q.from, effective_to),
                "cutoff_applied": cutoff_applied,
            })),
        )
            .into_response();
    }

    let sq = forward::SeriesQuery::new(
        q.venue.clone(),
        q.instrument.clone(),
        q.from,
        effective_to,
        effective_as_of,
    );
    let read = match forward::read_funding(&state.clickhouse_url, &sq, kind).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    let manifest = json!({
        "type": "funding",
        "venue": q.venue,
        "instrument": q.instrument,
        "kind": kind.as_str(),
        "window": {"from": q.from, "to": effective_to},
        "as_of": effective_as_of,
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
    });

    // The mapping from the store's type to the wire's is separated from the
    // shaping so the shaping can be tested. `FundingRate` is deliberately sealed --
    // "the only way to obtain one is to decode a row that exists", so that an
    // uncollected hour cannot be entered as an accrual of zero -- and that seal is
    // worth more than the convenience of building one in a test. So the test drives
    // `funding_body` over plain values instead, and this match stays a total,
    // field-for-field copy the compiler checks for exhaustiveness.
    let shape = match read {
        forward::SeriesRead::NoSuchSeries { table, database } => FundingShape::Absent {
            table: table.to_string(),
            database,
        },
        forward::SeriesRead::Present { rows } => FundingShape::Present(
            rows.iter()
                .map(|p| FundingRowOut {
                    event_time: p.event_time,
                    knowledge_time: p.knowledge_time,
                    rate: p.rate.get().to_string(),
                    relative_rate: p.relative_rate.map(|r| r.get().to_string()),
                    quality_flags: p.quality_flags.0,
                    source_id: p.source_id,
                    is_forward_evidence: p.is_forward_evidence(),
                })
                .collect(),
        ),
    };
    let logged_rows = match &shape {
        FundingShape::Absent { .. } => 0,
        FundingShape::Present(r) => r.len(),
    };
    let body = funding_body(manifest, shape);

    if logged_rows == 0 && body["funding"].is_null() {
        // Nothing was measured, so nothing was explored. Logging an exploration
        // here would put a row in the hypothesis registry saying somebody looked at
        // funding, when what happened is that there is no funding series to look at.
        return (StatusCode::OK, Json(body)).into_response();
    }

    if let Some(jobs) = state.jobs.as_ref() {
        let _ = jobs
            .log_exploration(jobs::ExplorationEntry {
                project_id: project.project_id,
                user_id: token.user_id(),
                session_id: None,
                source: if project.is_desk() {
                    "desk"
                } else {
                    "data_api"
                },
                instruments: vec![q.instrument.clone()],
                timeframe: None,
                window_start: Some(q.from),
                window_end: Some(effective_to),
                variables: vec!["funding_rate".into()],
                description: format!(
                    "funding {} {} {} [{}, {})",
                    q.venue,
                    q.instrument,
                    kind.as_str(),
                    q.from,
                    effective_to
                ),
                handle: None,
            })
            .await;
    }

    (StatusCode::OK, Json(body)).into_response()
}

/// One funding row as the wire carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FundingRowOut {
    pub event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    /// The venue's own digits, never an `f64` (INV-05).
    pub rate: String,
    pub relative_rate: Option<String>,
    pub quality_flags: u32,
    pub source_id: i32,
    /// False when the knowledge time was stamped by a backfill rather than
    /// received (D-P3). Gate 15 may not take evidence from such a row.
    pub is_forward_evidence: bool,
}

/// What the store had, reduced to what the wire needs.
///
/// Two variants because the store has two, and collapsing them is the defect this
/// route is built around.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FundingShape {
    /// The funding table is not in this database.
    Absent { table: String, database: String },
    /// The table exists and this is the window. An empty `Vec` here is a
    /// measurement.
    Present(Vec<FundingRowOut>),
}

/// The response body, as a pure function of the manifest and what was read.
///
/// Separated from the handler so that the one property worth pinning can be
/// tested without a ClickHouse: **an absent series answers `null` and an empty
/// window answers `[]`, and they are never the same value.** Funding is a cost, so
/// an unknown interval read as zero biases every carry-paying strategy upward --
/// it is the one series where the missing measurement looks like good news.
#[must_use]
pub fn funding_body(mut manifest: serde_json::Value, shape: FundingShape) -> serde_json::Value {
    match shape {
        FundingShape::Absent { table, database } => {
            manifest["measured"] = json!(false);
            manifest["unmeasured"] = json!({
                "reason": "no_such_series",
                "table": table,
                // Included because the last time somebody asked this question they
                // were connected to the wrong database.
                "database": database,
                "message": "this database has no funding series. That is not a funding rate of zero: a perpetual's carry over this window is unknown, and a study that needs it must skip the interval or refuse rather than treat the carry as free.",
            });
            // `null`, not `[]`. A caller writing `data.funding.length` throws
            // instead of reading `0`, and `?? []` becomes a decision somebody made.
            json!({"manifest": manifest, "funding": serde_json::Value::Null})
        }
        FundingShape::Present(rows) => {
            manifest["measured"] = json!(true);
            manifest["rows"] = json!(rows.len());
            // Stated per response rather than left to be counted: a window whose
            // rows are all backfilled is a window Gate 15 cannot take evidence
            // from, and that is invisible from the row count alone (D-P3).
            manifest["forward_evidence_rows"] =
                json!(rows.iter().filter(|r| r.is_forward_evidence).count());
            let out: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "event_time": r.event_time,
                        "knowledge_time": r.knowledge_time,
                        "rate": r.rate,
                        "relative_rate": r.relative_rate,
                        "quality_flags": r.quality_flags,
                        "source_id": r.source_id,
                        "is_forward_evidence": r.is_forward_evidence,
                    })
                })
                .collect();
            json!({"manifest": manifest, "funding": out})
        }
    }
}

/// `realized` (the default), `predicted`, or a refusal.
///
/// Not `unwrap_or(Realized)`. The two series answer different questions -- one is
/// settled and one is the venue's forecast -- and a typo silently bucketed as
/// realized is exactly the blending `FundingKind` exists to prevent.
fn parse_funding_kind(raw: Option<&str>) -> Result<forward::FundingKind, String> {
    match raw {
        None | Some("realized") => Ok(forward::FundingKind::Realized),
        Some("predicted") => Ok(forward::FundingKind::Predicted),
        Some(other) => Err(other.to_string()),
    }
}

// ── Coverage census (Set P P-0.2, P-0.6 clause 3) ─────────────────────────────

/// One census row as the coverage page needs it: the newest reading per series.
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct CoverageRow {
    pub instrument: String,
    pub venue: String,
    pub timeframe: String,
    pub as_of: DateTime<Utc>,
    pub period_secs: i32,
    pub series_kind: String,
    pub first_bar: Option<DateTime<Utc>>,
    pub last_bar: Option<DateTime<Utc>>,
    pub expected_bars: i64,
    pub present_bars: i64,
    pub missing_bps: i32,
    pub longest_continuous_secs: i64,
    pub derived_knowledge_bars: i64,
    pub observed_knowledge_bars: i64,
    pub strict_v1_satisfied: bool,
    pub strict_v1_shortfall_days: Option<i32>,
    pub paper_v1_satisfied: bool,
    pub paper_v1_shortfall_days: Option<i32>,
    pub last_row_age_secs: Option<i64>,
}

/// A series is *accruing* when its newest row is no older than this many periods
/// (and never less than a day, so a daily series is not called dead at 25 hours).
pub const ACCRUING_WITHIN_PERIODS: i64 = 3;

/// Whether the series is still growing: the condition under which "days until
/// the floor" means anything. A silent collector's shortfall does not close on its
/// own, and printing a countdown for it would be the reassurance that hides a dead
/// feed (Set P §8).
#[must_use]
pub fn is_accruing(row: &CoverageRow) -> bool {
    let window = (i64::from(row.period_secs) * ACCRUING_WITHIN_PERIODS).max(86_400);
    row.last_row_age_secs.is_some_and(|a| a <= window)
}

/// What one floor permits, in words.
#[must_use]
pub fn floor_sentence(
    floor: &str,
    floor_days: i64,
    satisfied: bool,
    shortfall_days: Option<i32>,
    accruing: bool,
    present_bars: i64,
) -> String {
    if satisfied {
        return format!("{floor} ({floor_days}-day floor): satisfiable now.");
    }
    if present_bars == 0 {
        return format!(
            "{floor} ({floor_days}-day floor): not satisfiable; the series holds no rows."
        );
    }
    let short =
        shortfall_days.map_or_else(|| "an unmeasured number of".to_string(), |d| d.to_string());
    if accruing {
        format!(
            "{floor} ({floor_days}-day floor): short by {short} days; at the current collection rate \
             (one day of depth per calendar day) it is satisfiable in {short} days, or sooner by backfill."
        )
    } else {
        format!(
            "{floor} ({floor_days}-day floor): short by {short} days, and the series is NOT accruing: \
             its newest row is stale, so the shortfall will not close without a backfill or a restarted collector."
        )
    }
}

/// The JSON for one coverage row, with the depth stated in words.
#[must_use]
pub fn coverage_row_json(row: &CoverageRow) -> serde_json::Value {
    let accruing = is_accruing(row);
    json!({
        "instrument": row.instrument,
        "venue": row.venue,
        "timeframe": row.timeframe,
        "series_kind": row.series_kind,
        "as_of": row.as_of,
        "first_bar": row.first_bar,
        "last_bar": row.last_bar,
        "expected_bars": row.expected_bars,
        "present_bars": row.present_bars,
        "missing_bps": row.missing_bps,
        "continuous_days": row.longest_continuous_secs / 86_400,
        "derived_knowledge_bars": row.derived_knowledge_bars,
        "observed_knowledge_bars": row.observed_knowledge_bars,
        "last_row_age_secs": row.last_row_age_secs,
        "accruing": accruing,
        "strict_v1": {
            "satisfied": row.strict_v1_satisfied,
            "shortfall_days": row.strict_v1_shortfall_days,
            "permits": floor_sentence(
                "strict_v1 Gate 9",
                dataplane::census::STRICT_V1_FLOOR_DAYS,
                row.strict_v1_satisfied,
                row.strict_v1_shortfall_days,
                accruing,
                row.present_bars,
            ),
        },
        "paper_v1": {
            "satisfied": row.paper_v1_satisfied,
            "shortfall_days": row.paper_v1_shortfall_days,
            "permits": floor_sentence(
                "paper_v1",
                dataplane::census::PAPER_V1_FLOOR_DAYS,
                row.paper_v1_satisfied,
                row.paper_v1_shortfall_days,
                accruing,
                row.present_bars,
            ),
        },
    })
}

/// Thinnest first: the series that decides what the platform can evaluate is the
/// one a reader must see first. Ties break on the series key so the order is
/// deterministic.
pub fn sort_thinnest_first(rows: &mut [CoverageRow]) {
    rows.sort_by(|a, b| {
        (
            a.longest_continuous_secs,
            &a.instrument,
            &a.venue,
            &a.timeframe,
        )
            .cmp(&(
                b.longest_continuous_secs,
                &b.instrument,
                &b.venue,
                &b.timeframe,
            ))
    });
}

/// One forward-only series' liveness, from the heartbeat table.
#[must_use]
pub fn forward_row_json(h: &forward::Heartbeat, now: DateTime<Utc>) -> serde_json::Value {
    json!({
        "series": h.series,
        "venue": h.venue,
        "instrument": h.instrument,
        "last_event_time": h.last_event_time,
        "last_write_time": h.last_ingest_time,
        "last_row_age_secs": (now - h.last_event_time).num_seconds(),
        "rows_written_last_beat": h.rows_written,
    })
}

#[derive(Debug, Deserialize)]
pub struct CoverageQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/coverage: the coverage census, thinnest series first (P-0.2).
///
/// **Desk only.** The census reports every series' true last bar, and for a
/// research project "history ends on the 4th" is information about the future,
/// the same reason `get_catalog` clips through the horizon. Rather than clip a
/// census whose whole point is the true depth, a non-Desk project is refused.
///
/// Forward-only series (P-0.6 clause 3) are listed beside the historical census
/// with their last-row age, read from `forward_series_heartbeat`. An absent
/// heartbeat table is reported as unmeasured, never as an empty list.
pub async fn get_coverage(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<CoverageQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    if !project.is_desk() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "desk_only",
                "message": "the coverage census states every series' true last bar, which a research project may not see past its cutoff",
                "fix": "re-issue this request against your Desk project (omit project_id)",
            })),
        )
            .into_response();
    }

    let rows: Result<Vec<CoverageRow>, sqlx::Error> = sqlx::query_as(
        "SELECT DISTINCT ON (instrument, venue, timeframe)
                instrument, venue, timeframe, as_of, period_secs, series_kind,
                first_bar, last_bar, expected_bars, present_bars, missing_bps,
                longest_continuous_secs, derived_knowledge_bars, observed_knowledge_bars,
                strict_v1_satisfied, strict_v1_shortfall_days,
                paper_v1_satisfied, paper_v1_shortfall_days, last_row_age_secs
           FROM dataplane.data_coverage_census
          ORDER BY instrument, venue, timeframe, as_of DESC",
    )
    .fetch_all(&state.pg)
    .await;
    let mut rows = match rows {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    sort_thinnest_first(&mut rows);

    let now = Utc::now();
    let forward_series = match forward::read_heartbeats(&state.clickhouse_url).await {
        Ok(forward::SeriesRead::Present { rows }) => json!({
            "measured": true,
            "series": rows.iter().map(|h| forward_row_json(h, now)).collect::<Vec<_>>(),
        }),
        Ok(forward::SeriesRead::NoSuchSeries { table, database }) => json!({
            "measured": false,
            "reason": format!("{table} is not in database {database:?}"),
            "series": serde_json::Value::Null,
        }),
        Err(e) => json!({
            "measured": false,
            "reason": e.to_string(),
            "series": serde_json::Value::Null,
        }),
    };

    let census_as_of = rows.iter().map(|r| r.as_of).max();
    (
        StatusCode::OK,
        Json(json!({
            "census_as_of": census_as_of,
            "census_taken": !rows.is_empty(),
            "floors": {
                "strict_v1_days": dataplane::census::STRICT_V1_FLOOR_DAYS,
                "paper_v1_days": dataplane::census::PAPER_V1_FLOOR_DAYS,
            },
            "series": rows.iter().map(coverage_row_json).collect::<Vec<_>>(),
            "forward": forward_series,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use chrono::TimeZone;

    fn row(name: &str, cont_days: i64, age: Option<i64>, strict_short: Option<i32>) -> CoverageRow {
        CoverageRow {
            instrument: name.into(),
            venue: "coinbase".into(),
            timeframe: "1m".into(),
            as_of: Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap(),
            period_secs: 60,
            series_kind: "historical".into(),
            first_bar: None,
            last_bar: None,
            expected_bars: 10,
            present_bars: 10,
            missing_bps: 0,
            longest_continuous_secs: cont_days * 86_400,
            derived_knowledge_bars: 0,
            observed_knowledge_bars: 10,
            strict_v1_satisfied: strict_short.is_none(),
            strict_v1_shortfall_days: strict_short,
            paper_v1_satisfied: true,
            paper_v1_shortfall_days: None,
            last_row_age_secs: age,
        }
    }

    /// P-0.2 clause 2: a series short of Gate 9 is labelled with its shortfall in days.
    #[test]
    fn a_series_short_of_gate_nine_states_its_shortfall_in_days() {
        let r = row("BTC-USD", 400, Some(60), Some(1426));
        let j = coverage_row_json(&r);
        let s = j["strict_v1"]["permits"].as_str().unwrap();
        assert!(s.contains("short by 1426 days"), "{s}");
        assert!(s.contains("satisfiable in 1426 days"), "{s}");
        assert_eq!(j["strict_v1"]["satisfied"], json!(false));
        assert_eq!(
            j["paper_v1"]["permits"],
            json!("paper_v1 (365-day floor): satisfiable now.")
        );
    }

    /// A stale series gets no countdown: that would be the reassurance that hides
    /// a dead collector.
    #[test]
    fn a_stale_series_is_not_given_a_countdown() {
        let r = row("ETH-USD", 30, Some(20 * 86_400), Some(1796));
        let s = coverage_row_json(&r)["strict_v1"]["permits"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(s.contains("NOT accruing"), "{s}");
        assert!(!s.contains("satisfiable in"), "{s}");
        assert!(!is_accruing(&row("X", 1, None, Some(1))));
    }

    /// P-0.2: thinnest first.
    #[test]
    fn rows_sort_thinnest_first() {
        let mut rows = vec![
            row("DEEP", 900, Some(0), Some(926)),
            row("THIN", 2, Some(0), Some(1824)),
        ];
        sort_thinnest_first(&mut rows);
        assert_eq!(rows[0].instrument, "THIN");
    }

    #[test]
    fn an_empty_series_is_never_called_satisfiable() {
        let s = floor_sentence("paper_v1", 365, false, None, false, 0);
        assert!(s.contains("holds no rows"), "{s}");
    }

    /// P-0.2 clause 3 (token refusal) is structural: the handler's signature takes
    /// a `BearerToken`, whose extractor rejects a request without a valid token
    /// before the body runs. Pinned by source so removing the extractor fails here.
    #[test]
    fn the_coverage_handler_requires_a_bearer_token() {
        let src = include_str!("data.rs");
        let at = src.find("pub async fn get_coverage(").unwrap();
        let sig = &src[at..at + 200];
        assert!(sig.contains("token: BearerToken"), "{sig}");
    }
}

// ── As-of universes (Set P P-4.3, P-4.4) ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UniverseQuery {
    pub as_of: DateTime<Utc>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/universe/{name}?as_of= — membership known at `as_of` (DA-08).
///
/// Resolved through `backtest::env::universe::load`, the same loader every
/// backtest and live allocator uses, and asked at `as_of` clipped to the project's
/// horizon — so the members a caller sees here are the members a study at that
/// instant was evaluated on, on both of the log's clocks (`effective` and
/// `recorded_at`). Each member carries its current listing `status`, so an
/// instrument since delisted (P-4.4) is returned *and* labelled rather than
/// silently dropped, which is the survivorship bias this endpoint exists to avoid.
pub async fn get_universe(
    State(state): State<AppState>,
    token: BearerToken,
    Path(name): Path<String>,
    Query(q): Query<UniverseQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (as_of, cutoff_applied) = project.clip(q.as_of);
    let membership = match backtest::env::universe::load(
        &state.pg,
        &domain::strategy_graph::UniverseRef::Named { name: name.clone() },
    )
    .await
    {
        Ok(m) => m,
        Err(backtest::env::universe::UniverseLoadError::UnknownUniverse(n)) => return (
            StatusCode::NOT_FOUND,
            Json(
                json!({"error": "unknown_universe", "message": format!("no universe named {n:?}")}),
            ),
        )
            .into_response(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "universe_load_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let at_ns = as_of.timestamp_nanos_opt().unwrap_or(i64::MAX);
    let members = match membership.members_at(at_ns) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "universe_inconsistent", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let statuses: Vec<(String, String)> = sqlx::query_as(
        "SELECT instrument_id, status FROM instruments WHERE instrument_id = ANY($1)",
    )
    .bind(&members)
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();
    let status_of = |m: &str| {
        statuses
            .iter()
            .find(|(i, _)| i == m)
            .map_or("unknown", |(_, s)| s.as_str())
            .to_string()
    };
    (
        StatusCode::OK,
        Json(json!({
            "manifest": {
                "type": "universe",
                "name": name,
                "as_of": as_of,
                "cutoff_applied": cutoff_applied,
                "project_id": project.project_id,
                "rows": members.len(),
            },
            "members": members.iter().map(|m| json!({
                "instrument": m,
                "status": status_of(m),
            })).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

// ── Alternative data: macro, prediction markets, text (Set P P-3.1, P-3.4, P-3.5) ──
//
// Each route clips `as_of` to the project's horizon before it reads, and reads
// through `storage::clickhouse::alt`, whose every query takes that `as_of`. There
// is no route here that reads one of these tables without a cutoff.

use storage::clickhouse::alt;

/// The scope a token must hold to read raw text (DATA-005 §5, DA-09). Text is the
/// one series an agent can be steered by, so it is read only through the reader
/// role and never through `research:data.read`.
pub const TEXT_READER_SCOPE: &str = crate::auth::scopes::TEXT_READER;

/// Whether a scope set may read raw text. `web:full` (a human's login) may; every
/// research scope other than the reader may not.
#[must_use]
pub fn may_read_text(scopes: &[String]) -> bool {
    crate::auth::scopes::permits(scopes, TEXT_READER_SCOPE)
}

fn bad_request(code: &str, message: String) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": code, "message": message})),
    )
        .into_response()
}

fn query_failed(e: impl std::fmt::Display) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "query_failed", "message": e.to_string()})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct MacroQuery {
    pub series: String,
    pub start: chrono::NaiveDate,
    pub end: chrono::NaiveDate,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/macro — an ALFRED series as it stood at `as_of` (P-3.1).
pub async fn get_macro(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<MacroQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    if q.start > q.end {
        return bad_request(
            "empty_window",
            format!("start {} is after end {}", q.start, q.end),
        );
    }
    let (as_of, cutoff_applied) = project.clip(q.as_of.unwrap_or_else(Utc::now));
    match alt::macro_as_of(&state.clickhouse_url, &q.series, q.start, q.end, as_of).await {
        Ok(points) => (
            StatusCode::OK,
            Json(json!({
                "manifest": {
                    "type": "macro",
                    "series": q.series,
                    "window": {"start": q.start, "end": q.end},
                    "as_of": as_of,
                    "cutoff_applied": cutoff_applied,
                    "revision_policy": "latest_vintage_as_of",
                    "project_id": project.project_id,
                    "rows": points.len(),
                },
                "observations": points,
            })),
        )
            .into_response(),
        Err(e) => query_failed(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PredictionQuery {
    pub ticker: String,
    #[serde(default = "default_prediction_venue")]
    pub venue: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

fn default_prediction_venue() -> String {
    "kalshi".into()
}

/// GET /api/data/prediction_markets — event probabilities as known at `as_of`
/// (P-3.4). Exposure only: this is a read; nothing in the platform trades it.
pub async fn get_prediction_markets(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<PredictionQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (end, cutoff_applied) = project.clip(q.end);
    let (as_of, _) = project.clip(q.as_of.unwrap_or(end));
    if q.start >= end {
        return bad_request(
            "empty_window",
            format!("start {} is at or after the effective end {end}", q.start),
        );
    }
    match alt::probabilities_as_of(&state.clickhouse_url, &q.venue, &q.ticker, q.start, end, as_of).await {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "manifest": {
                    "type": "prediction_markets",
                    "venue": q.venue,
                    "ticker": q.ticker,
                    "window": {"start": q.start, "end": end},
                    "as_of": as_of,
                    "cutoff_applied": cutoff_applied,
                    "project_id": project.project_id,
                    "rows": rows.len(),
                    "forward_evidence_rows": rows.iter().filter(|r| r.quality_flags & 0x8000 == 0).count(),
                },
                "probabilities": rows,
            })),
        )
            .into_response(),
        Err(e) => query_failed(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct TextQuery {
    #[serde(default)]
    pub instrument: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/text — text items as they stood at `as_of` (P-3.5).
///
/// Refused with 403 unless the token holds [`TEXT_READER_SCOPE`]. `as_of` and the
/// window are both clipped to the project horizon, so no item whose availability
/// exceeds the cutoff is ever returned.
pub async fn get_text(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<TextQuery>,
) -> impl IntoResponse {
    if !may_read_text(&token.scopes) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "text_reader_only",
                "message": format!("raw text is readable only with the {TEXT_READER_SCOPE} scope"),
            })),
        )
            .into_response();
    }
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (end, cutoff_applied) = project.clip(q.end);
    let (as_of, _) = project.clip(q.as_of.unwrap_or(end));
    let limit = q.limit.unwrap_or(200).clamp(1, 1_000);
    match alt::text_as_of(
        &state.clickhouse_url,
        q.instrument.as_deref(),
        q.start,
        end,
        as_of,
        limit,
    )
    .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(json!({
                "manifest": {
                    "type": "text",
                    "instrument": q.instrument,
                    "window": {"start": q.start, "end": end},
                    "as_of": as_of,
                    "cutoff_applied": cutoff_applied,
                    "revision_policy": "latest_revision_available_as_of",
                    "project_id": project.project_id,
                    "rows": items.len(),
                },
                "items": items,
            })),
        )
            .into_response(),
        Err(e) => query_failed(e),
    }
}

#[cfg(test)]
mod alt_route_tests {
    use super::*;

    /// P-3.5 clause 2: every research scope other than the reader is refused text,
    /// one assertion per scope, so a scope added later is checked by name.
    #[test]
    fn no_scope_but_the_reader_may_read_text() {
        for s in crate::auth::scopes::RESEARCH_SCOPES {
            let held = vec![(*s).to_string()];
            assert_eq!(
                may_read_text(&held),
                *s == TEXT_READER_SCOPE,
                "scope {s} must not read raw text"
            );
        }
        for s in [
            "research:data.read",
            "research:features",
            "llm:proxy",
            "monitor.control",
            "",
        ] {
            assert!(!may_read_text(&[s.to_string()]), "{s}");
        }
        assert!(may_read_text(&[TEXT_READER_SCOPE.to_string()]));
        // A human's web login holds everything, by design of `permits`.
        assert!(may_read_text(&["web:full".to_string()]));
    }

    #[test]
    fn the_text_handler_checks_the_scope_before_it_reads() {
        let src = include_str!("data.rs");
        let at = src.find("pub async fn get_text(").unwrap();
        let body = &src[at..];
        let gate = body.find("may_read_text(&token.scopes)").unwrap();
        let read = body.find("alt::text_as_of(").unwrap();
        assert!(gate < read, "the scope check must precede the read");
    }
}

// ── Derived bars: server-side resampling and information-driven bars (P-4.1, P-4.2) ──

#[derive(Debug, Deserialize)]
pub struct DerivedBarsQuery {
    pub instrument: String,
    pub venue: String,
    /// The stored timeframe to derive from.
    #[serde(default = "default_timeframe")]
    pub from_tf: String,
    /// A coarser clock timeframe (`1h`, `1d`, …) — set this, or `kind`.
    #[serde(default)]
    pub tf: Option<String>,
    /// `volume` | `dollar` | `tick` — information-driven bars, with `threshold`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub threshold: Option<rust_decimal::Decimal>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

fn tf_secs(tf: &str) -> Option<u32> {
    match tf {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        _ => None,
    }
}

/// Which derivation a request asks for, or why it is malformed.
#[derive(Debug, PartialEq, Eq)]
pub enum Derivation {
    Resample { to_secs: u32 },
    Information(dataplane::bars::information_driven::InformationBarSpec),
}

/// Parse the request's derivation. Exactly one of `tf` and `kind`.
///
/// # Errors
/// Both or neither given, an unknown timeframe or kind, or a kind without a
/// threshold.
pub fn derivation_of(q: &DerivedBarsQuery) -> Result<Derivation, String> {
    use dataplane::bars::information_driven::{InformationBarKind, InformationBarSpec};
    match (&q.tf, &q.kind) {
        (Some(tf), None) => tf_secs(tf)
            .map(|to_secs| Derivation::Resample { to_secs })
            .ok_or_else(|| format!("unknown timeframe {tf:?}")),
        (None, Some(kind)) => {
            let kind = match kind.as_str() {
                "volume" => InformationBarKind::Volume,
                "dollar" => InformationBarKind::Dollar,
                "tick" => InformationBarKind::Tick,
                other => {
                    return Err(format!(
                        "unknown bar kind {other:?}; volume, dollar or tick"
                    ))
                }
            };
            let threshold = q
                .threshold
                .ok_or("an information-driven bar needs a threshold, and none is assumed")?;
            Ok(Derivation::Information(InformationBarSpec::new(
                kind, threshold,
            )))
        }
        (Some(_), Some(_)) => {
            Err("give `tf` (resample) or `kind` (information bars), not both".into())
        }
        (None, None) => {
            Err("give `tf` (resample to a coarser clock) or `kind` (volume|dollar|tick)".into())
        }
    }
}

/// GET /api/data/bars/derived — read-time derivations over stored bars.
///
/// The input is read point-in-time **with its provenance**
/// (`storage::clickhouse::canonical_read`), and the derivation inherits the
/// **worst** input grade and the **largest** declared lag; a series nobody has
/// graded is refused by the derivation itself rather than assigned a letter here.
/// A finer timeframe is refused, never interpolated.
pub async fn get_derived_bars(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<DerivedBarsQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let derivation = match derivation_of(&q) {
        Ok(d) => d,
        Err(m) => return bad_request("invalid_derivation", m),
    };
    let Some(from_secs) = tf_secs(&q.from_tf) else {
        return bad_request(
            "unknown_timeframe",
            format!("unknown timeframe {:?}", q.from_tf),
        );
    };
    let (end, cutoff_applied) = project.clip(q.end);
    let (as_of, _) = project.clip(q.as_of.unwrap_or(end));
    if q.start >= end {
        return bad_request(
            "empty_window",
            format!("start {} is at or after the effective end {end}", q.start),
        );
    }

    let bars = match storage::clickhouse::canonical_read::canonical_bars_as_of(
        &state.clickhouse_url,
        &q.instrument,
        &q.venue,
        from_secs,
        q.start,
        end,
        as_of,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => return query_failed(e),
    };
    let grade =
        match storage::clickhouse::qc_reports::latest_grade(&state.pg, &q.instrument, &q.from_tf)
            .await
        {
            Ok(g) => g.and_then(|g| storage::clickhouse::canonical_read::parse_grade(&g.grade)),
            Err(e) => return query_failed(e),
        };
    let provenance =
        match storage::clickhouse::canonical_read::provenance(&state.pg, &bars, grade).await {
            Ok(p) => p,
            Err(e) => return query_failed(e),
        };

    let manifest = json!({
        "type": "derived_bars",
        "instrument": q.instrument,
        "venue": q.venue,
        "from_timeframe": q.from_tf,
        "window": {"start": q.start, "end": end},
        "as_of": as_of,
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "input_rows": bars.len(),
    });
    let body = match derivation {
        Derivation::Resample { to_secs } => dataplane::resample::resample(
            &bars,
            to_secs,
            &provenance,
            dataplane::resample::ResampleOptions::default(),
        )
        .map(|s| json!({"manifest": manifest, "series": s}))
        .map_err(|e| e.to_string()),
        Derivation::Information(spec) => {
            dataplane::bars::information_driven::generate(spec, &bars, &provenance)
                .map(|s| json!({"manifest": manifest, "series": s}))
                .map_err(|e| e.to_string())
        }
    };
    match body {
        Ok(b) => (StatusCode::OK, Json(b)).into_response(),
        // A refusal from the derivation is a stated reason, never an empty series.
        Err(reason) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "derivation_refused", "message": reason, "manifest": manifest})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod derived_tests {
    use super::*;

    fn q(tf: Option<&str>, kind: Option<&str>, threshold: Option<i64>) -> DerivedBarsQuery {
        DerivedBarsQuery {
            instrument: "BTC-USD".into(),
            venue: "coinbase".into(),
            from_tf: "1m".into(),
            tf: tf.map(Into::into),
            kind: kind.map(Into::into),
            threshold: threshold.map(rust_decimal::Decimal::from),
            start: Utc::now(),
            end: Utc::now(),
            as_of: None,
            project_id: None,
        }
    }

    #[test]
    fn a_request_names_exactly_one_derivation() {
        assert_eq!(
            derivation_of(&q(Some("1h"), None, None)),
            Ok(Derivation::Resample { to_secs: 3600 })
        );
        assert!(derivation_of(&q(Some("1h"), Some("volume"), Some(5))).is_err());
        assert!(derivation_of(&q(None, None, None)).is_err());
        assert!(derivation_of(&q(Some("7m"), None, None)).is_err());
    }

    #[test]
    fn an_information_bar_needs_its_threshold_stated() {
        assert!(derivation_of(&q(None, Some("volume"), None)).is_err());
        assert!(derivation_of(&q(None, Some("candles"), Some(5))).is_err());
        assert!(matches!(
            derivation_of(&q(None, Some("dollar"), Some(1_000_000))),
            Ok(Derivation::Information(_))
        ));
    }
}

// ── Set L Phase 2: the remaining DATA-005 §5 endpoints, Parquet, snapshot ids ──
//
// (track LEGACY, 2026-10-03). Everything here goes through the same three steps
// as `get_bars`: resolve the project, clip `end` and `as_of` to its horizon, and
// answer with a manifest that states what was clipped. Nothing reads a table
// without an `as_of`.
//
// Two things are shared by every endpoint in this block and by `get_bars`:
//
// * `data_snapshot_id` -- a content address for *which version of the data* a
//   read saw: the endpoint, its normalised parameters, the `as_of`, and a digest
//   of the rows returned. Two reads with the same id saw the same data, and a late
//   correction changes the id. A job manifest can therefore pin it.
// * `?format=parquet` -- the rows are encoded as Parquet (prices and sizes as
//   `decimal128(38,10)`, ADR-0002), stored as a `parquet_extract` artifact, and
//   the response is `{handle, manifest, summary}` instead of the rows.

use std::sync::Arc;

/// Column encodings for a Parquet extract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PqCol {
    /// Unix nanoseconds, written as `timestamp(ns, UTC)`.
    TsNs,
    I64,
    /// A derived statistic. The only place a float is allowed (DATA-005 §6).
    F64,
    /// A decimal carried as a string in the JSON rows, written as `decimal128(38,10)`.
    Dec,
    Str,
}

/// Encode JSON rows as Parquet. Every column is nullable: an absent measurement
/// is a null, never a zero (INV-05, ADR-0002).
///
/// # Errors
/// A decimal that will not parse or does not fit `decimal128(38,10)`, or the writer.
pub fn rows_to_parquet(
    cols: &[(&str, PqCol)],
    rows: &[serde_json::Value],
) -> anyhow::Result<Vec<u8>> {
    use arrow::array::{
        ArrayRef, Decimal128Array, Float64Array, Int64Array, StringArray, TimestampNanosecondArray,
    };
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;

    let mut fields = Vec::with_capacity(cols.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(cols.len());
    for (name, kind) in cols {
        let cell = |r: &serde_json::Value| r.get(*name).filter(|v| !v.is_null()).cloned();
        match kind {
            PqCol::TsNs => {
                fields.push(Field::new(
                    *name,
                    DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                    true,
                ));
                let v: Vec<Option<i64>> = rows
                    .iter()
                    .map(|r| cell(r).and_then(|c| c.as_i64()))
                    .collect();
                arrays.push(Arc::new(
                    TimestampNanosecondArray::from(v).with_timezone("UTC"),
                ));
            }
            PqCol::I64 => {
                fields.push(Field::new(*name, DataType::Int64, true));
                let v: Vec<Option<i64>> = rows
                    .iter()
                    .map(|r| cell(r).and_then(|c| c.as_i64()))
                    .collect();
                arrays.push(Arc::new(Int64Array::from(v)));
            }
            PqCol::F64 => {
                fields.push(Field::new(*name, DataType::Float64, true));
                let v: Vec<Option<f64>> = rows
                    .iter()
                    .map(|r| cell(r).and_then(|c| c.as_f64()))
                    .collect();
                arrays.push(Arc::new(Float64Array::from(v)));
            }
            PqCol::Dec => {
                fields.push(Field::new(*name, DataType::Decimal128(38, 10), true));
                let mut v: Vec<Option<i128>> = Vec::with_capacity(rows.len());
                for r in rows {
                    v.push(match cell(r) {
                        None => None,
                        Some(c) => {
                            let text = c
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| c.to_string());
                            Some(decimal_to_i128_scale10(&text)?)
                        }
                    });
                }
                arrays.push(Arc::new(
                    Decimal128Array::from(v).with_precision_and_scale(38, 10)?,
                ));
            }
            PqCol::Str => {
                fields.push(Field::new(*name, DataType::Utf8, true));
                let v: Vec<Option<String>> = rows
                    .iter()
                    .map(|r| {
                        cell(r).map(|c| match c {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        })
                    })
                    .collect();
                arrays.push(Arc::new(StringArray::from(v)));
            }
        }
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = ArrowWriter::try_new(&mut buf, schema, None)?;
        w.write(&batch)?;
        w.close()?;
    }
    Ok(buf)
}

/// A decimal string as the unscaled `i128` of `decimal128(38,10)`.
fn decimal_to_i128_scale10(text: &str) -> anyhow::Result<i128> {
    let d: rust_decimal::Decimal = text
        .parse()
        .map_err(|e| anyhow::anyhow!("decimal {text:?}: {e}"))?;
    let d = d.round_dp(10);
    Ok(d.mantissa() * 10_i128.pow(10 - d.scale()))
}

/// The content address of one read (DATA-005 §6). `rows_digest` is the sha256 of
/// the rows as served, so a restated value changes the id even when every
/// parameter is the same.
#[must_use]
pub fn data_snapshot_id(
    endpoint: &str,
    params: &serde_json::Value,
    as_of: DateTime<Utc>,
    rows_digest: &str,
) -> String {
    let descriptor = json!({
        "endpoint": endpoint,
        "params": params,
        "as_of": as_of.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        "rows_sha256": rows_digest,
    });
    let hash = jobs::manifest::sha256_hex(jobs::manifest::canonical(&descriptor).as_bytes());
    format!("ds_{}", &hash[..32])
}

/// The one way every endpoint in this block (and `get_bars`) answers.
///
/// Stamps `data_snapshot_id` and a recomputable `snapshot` descriptor into the
/// manifest, then returns either the rows (JSON) or a `parquet_extract` handle.
#[allow(clippy::too_many_arguments)]
async fn deliver(
    state: &AppState,
    project: &Project,
    endpoint: &str,
    params: serde_json::Value,
    as_of: DateTime<Utc>,
    mut manifest: serde_json::Value,
    rows_key: &str,
    rows: Vec<serde_json::Value>,
    cols: &[(&str, PqCol)],
    format: Option<&str>,
) -> axum::response::Response {
    let digest = jobs::manifest::sha256_hex(&serde_json::to_vec(&rows).unwrap_or_default());
    let snapshot_id = data_snapshot_id(endpoint, &params, as_of, &digest);
    manifest["data_snapshot_id"] = json!(snapshot_id);
    manifest["snapshot"] = json!({
        "endpoint": endpoint,
        "params": params,
        "as_of": as_of,
        "rows_sha256": digest,
    });
    manifest["rows"] = json!(rows.len());

    match format.unwrap_or("json") {
        "json" => {
            let mut body = json!({ "manifest": manifest });
            body[rows_key] = serde_json::Value::Array(rows);
            (StatusCode::OK, Json(body)).into_response()
        }
        "parquet" => {
            let Some(registry) = state.artifacts.as_ref() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": "artifacts_unavailable",
                        "message": "the artifact store is not configured on this server",
                        "fix": "request format=json, or start the server with the job service enabled",
                    })),
                )
                    .into_response();
            };
            let bytes = match rows_to_parquet(cols, &rows) {
                Ok(b) => b,
                Err(e) => return query_failed(format!("parquet encode: {e}")),
            };
            manifest["schema"] = json!(cols
                .iter()
                .map(|(n, k)| json!({"name": n, "encoding": format!("{k:?}")}))
                .collect::<Vec<_>>());
            manifest["format"] = json!("parquet");
            match registry
                .put(
                    "parquet_extract",
                    Some(project.project_id),
                    &bytes,
                    manifest.clone(),
                    None,
                )
                .await
            {
                Ok(artifact) => {
                    let summary = [
                        format!("{endpoint}: {} rows", rows.len()),
                        format!("window: {}", manifest["window"]),
                        format!("cutoff_applied: {}", manifest["cutoff_applied"]),
                        format!(
                            "qc_grade: {}",
                            manifest.get("qc_grade").unwrap_or(&json!("n/a"))
                        ),
                        format!("data_snapshot_id: {snapshot_id}"),
                        format!("handle: {}", artifact.handle),
                    ]
                    .join("\n");
                    (
                        StatusCode::OK,
                        Json(json!({
                            "handle": artifact.handle,
                            "manifest": manifest,
                            "summary": summary,
                        })),
                    )
                        .into_response()
                }
                Err(e) => query_failed(format!("artifact store: {e}")),
            }
        }
        other => bad_request(
            "unknown_format",
            format!("unknown format {other:?}; json or parquet"),
        ),
    }
}

/// Every read is exploration, and exploration is logged rather than counted
/// (D-13, JB-11) -- the same entry `get_bars` writes.
#[allow(clippy::too_many_arguments)]
async fn log_data_read(
    state: &AppState,
    project: &Project,
    user_id: Uuid,
    instruments: Vec<String>,
    window: (DateTime<Utc>, DateTime<Utc>),
    variables: &[&str],
    description: String,
) {
    if let Some(jobs) = state.jobs.as_ref() {
        let _ = jobs
            .log_exploration(jobs::ExplorationEntry {
                project_id: project.project_id,
                user_id,
                session_id: None,
                source: if project.is_desk() {
                    "desk"
                } else {
                    "data_api"
                },
                instruments,
                timeframe: None,
                window_start: Some(window.0),
                window_end: Some(window.1),
                variables: variables.iter().map(|v| (*v).to_string()).collect(),
                description,
                handle: None,
            })
            .await;
    }
}

/// The clipped window every windowed endpoint here uses. `Err` is the ready
/// 422 response for an empty window.
#[allow(clippy::result_large_err)]
fn clip_window(
    project: &Project,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    as_of: Option<DateTime<Utc>>,
) -> Result<(DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>), axum::response::Response> {
    let (effective_end, cutoff_applied) = project.clip(end);
    let (effective_as_of, _) = project.clip(as_of.unwrap_or(effective_end));
    if start >= effective_end {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_window",
                "message": format!("start {start} is at or after the effective end {effective_end}"),
                "cutoff_applied": cutoff_applied,
            })),
        )
            .into_response());
    }
    Ok((effective_end, effective_as_of, cutoff_applied))
}

fn ch_ns(t: DateTime<Utc>) -> String {
    format!(
        "fromUnixTimestamp64Nano(toInt64({}))",
        t.timestamp_nanos_opt().unwrap_or(i64::MAX)
    )
}

fn ch_esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// A ClickHouse error that means the table does not exist in this database. An
/// absent series is reported as unmeasured, never as an empty one (see `get_funding`).
fn is_missing_table(message: &str) -> bool {
    message.contains("UNKNOWN_TABLE")
        || message.contains("doesn't exist")
        || message.contains("Code: 60")
}

fn unmeasured_body(
    mut manifest: serde_json::Value,
    rows_key: &str,
    table: &str,
) -> serde_json::Value {
    manifest["measured"] = json!(false);
    manifest["unmeasured"] = json!({
        "reason": "no_such_series",
        "table": table,
        "message": "nothing has been collected for this series in this database; an empty list would read as 'measured and found nothing'",
    });
    let mut body = json!({ "manifest": manifest });
    body[rows_key] = serde_json::Value::Null;
    body
}

// ── open_interest ────────────────────────────────────────────────────────────

const OI_COLS: &[(&str, PqCol)] = &[
    ("event_time_ns", PqCol::TsNs),
    ("knowledge_time_ns", PqCol::TsNs),
    ("open_interest", PqCol::Dec),
    ("mark_price", PqCol::Dec),
    ("quality_flags", PqCol::I64),
    ("is_forward_evidence", PqCol::Str),
];

#[derive(Debug, Deserialize)]
pub struct OpenInterestQuery {
    pub venue: String,
    pub instrument: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/open_interest — forward-collected open interest (DATA-005 §5).
///
/// Same three outcomes as `get_funding`: an absent series is `null`, an empty
/// window is `[]`, rows carry their own knowledge time.
pub async fn get_open_interest(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<OpenInterestQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (effective_to, effective_as_of, cutoff_applied) =
        match clip_window(&project, q.from, q.to, q.as_of) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
    let sq = forward::SeriesQuery::new(
        q.venue.clone(),
        q.instrument.clone(),
        q.from,
        effective_to,
        effective_as_of,
    );
    let read = match forward::read_open_interest(&state.clickhouse_url, &sq).await {
        Ok(r) => r,
        Err(e) => return query_failed(e),
    };
    let manifest = json!({
        "type": "open_interest",
        "venue": q.venue,
        "instrument": q.instrument,
        "window": {"from": q.from, "to": effective_to},
        "as_of": effective_as_of,
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
    });
    let rows = match read {
        forward::SeriesRead::NoSuchSeries { table, .. } => {
            return (
                StatusCode::OK,
                Json(unmeasured_body(manifest, "open_interest", table)),
            )
                .into_response()
        }
        forward::SeriesRead::Present { rows } => rows,
    };
    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|p| {
            json!({
                "event_time_ns": p.event_time.timestamp_nanos_opt(),
                "knowledge_time_ns": p.knowledge_time.timestamp_nanos_opt(),
                "open_interest": p.open_interest.get().to_string(),
                "mark_price": p.mark_price.map(|m| m.to_string()),
                "quality_flags": p.quality_flags.0,
                "is_forward_evidence": !p.quality_flags.contains(dataplane::quality::QualityFlags::BACKFILLED_KNOWLEDGE_TIME),
            })
        })
        .collect();
    log_data_read(
        &state,
        &project,
        token.user_id(),
        vec![q.instrument.clone()],
        (q.from, effective_to),
        &["open_interest"],
        format!(
            "open_interest {} {} [{}, {})",
            q.venue, q.instrument, q.from, effective_to
        ),
    )
    .await;
    deliver(
        &state,
        &project,
        "open_interest",
        json!({"venue": q.venue, "instrument": q.instrument, "from": q.from, "to": effective_to}),
        effective_as_of,
        manifest,
        "open_interest",
        out,
        OI_COLS,
        q.format.as_deref(),
    )
    .await
}

// ── trades ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TradesQuery {
    pub instrument: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

#[derive(clickhouse::Row, Deserialize)]
struct TradeWire {
    ts_ns: i64,
    price_s: String,
    size_s: String,
    side: String,
    venue_id: String,
    exchange_trade_id: String,
}

const TRADE_COLS: &[(&str, PqCol)] = &[
    ("ts_ns", PqCol::TsNs),
    ("price", PqCol::Dec),
    ("size", PqCol::Dec),
    ("side", PqCol::Str),
    ("venue_id", PqCol::Str),
    ("exchange_trade_id", PqCol::Str),
];

/// The SQL for one trades window. Separated so its clauses can be asserted.
///
/// `LIMIT 1 BY dedup_key` is the read-time half of the `ReplacingMergeTree`
/// dedup: until a background merge runs, a re-delivered trade is two rows.
#[must_use]
pub fn trades_sql(
    instrument: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    as_of: DateTime<Utc>,
    limit: usize,
) -> String {
    format!(
        "SELECT toUnixTimestamp64Nano(available_time) AS ts_ns, toString(price) AS price_s, \
                toString(size) AS size_s, side, venue_id, exchange_trade_id \
           FROM market_trades \
          WHERE instrument_id = '{}' \
            AND available_time >= {} AND available_time < {} AND available_time <= {} \
          ORDER BY available_time, dedup_key \
          LIMIT 1 BY dedup_key \
          LIMIT {}",
        ch_esc(instrument),
        ch_ns(from),
        ch_ns(to),
        ch_ns(as_of),
        limit + 1
    )
}

/// GET /api/data/trades — raw trades where they were collected (DATA-005 §5).
pub async fn get_trades(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<TradesQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (effective_end, effective_as_of, cutoff_applied) =
        match clip_window(&project, q.start, q.end, q.as_of) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
    let limit = q.limit.unwrap_or(10_000).clamp(1, 100_000) as usize;
    let manifest = json!({
        "type": "trades",
        "instrument": q.instrument,
        "window": {"start": q.start, "end": effective_end},
        "as_of": effective_as_of,
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
        "qc_grade": "ungraded",
    });
    let client = storage::clickhouse::connect(&state.clickhouse_url);
    let sql = trades_sql(
        &q.instrument,
        q.start,
        effective_end,
        effective_as_of,
        limit,
    );
    let mut wire: Vec<TradeWire> = match client.query(&sql).fetch_all().await {
        Ok(w) => w,
        Err(e) if is_missing_table(&e.to_string()) => {
            return (
                StatusCode::OK,
                Json(unmeasured_body(manifest, "trades", "market_trades")),
            )
                .into_response()
        }
        Err(e) => return query_failed(e),
    };
    let truncated = wire.len() > limit;
    wire.truncate(limit);
    let mut manifest = manifest;
    manifest["truncated"] = json!(truncated);
    let rows: Vec<serde_json::Value> = wire
        .into_iter()
        .map(|t| {
            json!({
                "ts_ns": t.ts_ns, "price": t.price_s, "size": t.size_s, "side": t.side,
                "venue_id": t.venue_id, "exchange_trade_id": t.exchange_trade_id,
            })
        })
        .collect();
    log_data_read(
        &state,
        &project,
        token.user_id(),
        vec![q.instrument.clone()],
        (q.start, effective_end),
        &["price", "size", "side"],
        format!("trades {} [{}, {})", q.instrument, q.start, effective_end),
    )
    .await;
    deliver(
        &state,
        &project,
        "trades",
        json!({"instrument": q.instrument, "start": q.start, "end": effective_end, "limit": limit}),
        effective_as_of,
        manifest,
        "trades",
        rows,
        TRADE_COLS,
        q.format.as_deref(),
    )
    .await
}

// ── option_chain ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OptionChainQuery {
    pub underlying: String,
    #[serde(default)]
    pub venue: Option<String>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

#[derive(clickhouse::Row, Deserialize)]
struct ChainWire {
    instrument: String,
    expiry_s: String,
    strike_s: String,
    opt_right: String,
    event_ns: i64,
    knowledge_ns: i64,
    bid_s: Option<String>,
    ask_s: Option<String>,
    mark_s: Option<String>,
    underlying_s: Option<String>,
    volume: Option<i64>,
    open_interest: Option<i64>,
    iv: Option<f64>,
    iv_model: Option<String>,
    iv_rate: Option<f64>,
    iv_div: Option<f64>,
    in_liquid_universe: u8,
}

const CHAIN_COLS: &[(&str, PqCol)] = &[
    ("instrument", PqCol::Str),
    ("expiry", PqCol::Str),
    ("strike", PqCol::Dec),
    ("right", PqCol::Str),
    ("event_time_ns", PqCol::TsNs),
    ("knowledge_time_ns", PqCol::TsNs),
    ("bid", PqCol::Dec),
    ("ask", PqCol::Dec),
    ("mark", PqCol::Dec),
    ("underlying_price", PqCol::Dec),
    ("volume", PqCol::I64),
    ("open_interest", PqCol::I64),
    ("iv", PqCol::F64),
    ("iv_model", PqCol::Str),
    ("delta", PqCol::F64),
    ("gamma", PqCol::F64),
    ("vega", PqCol::F64),
    ("theta", PqCol::F64),
    ("rho", PqCol::F64),
];

/// The SQL for the newest chain snapshot knowable at `as_of`.
#[must_use]
pub fn chain_sql(underlying: &str, venue: Option<&str>, as_of: DateTime<Utc>) -> String {
    let venue_clause = venue.map_or_else(String::new, |v| format!("AND venue = '{}' ", ch_esc(v)));
    let known = format!(
        "underlying = '{u}' {venue_clause}AND knowledge_time <= {t} AND event_time <= {t}",
        u = ch_esc(underlying),
        t = ch_ns(as_of),
    );
    format!(
        "SELECT instrument, toString(expiry) AS expiry_s, toString(strike) AS strike_s, \
                toString(option_right) AS opt_right, \
                toUnixTimestamp64Nano(event_time) AS event_ns, \
                toUnixTimestamp64Nano(knowledge_time) AS knowledge_ns, \
                toString(bid) AS bid_s, toString(ask) AS ask_s, toString(mark) AS mark_s, \
                toString(underlying_price) AS underlying_s, volume, open_interest, \
                iv, iv_model, iv_rate, iv_div, in_liquid_universe \
           FROM option_chain_snapshots \
          WHERE {known} \
            AND event_time = (SELECT max(event_time) FROM option_chain_snapshots WHERE {known}) \
          ORDER BY expiry, option_right, strike, knowledge_time DESC \
          LIMIT 1 BY instrument"
    )
}

/// Black-Scholes-Merton greeks from the stored inputs, at read time (INV-07, AT-08).
/// `None` for a contract without the inputs: a missing IV is not a zero greek.
#[must_use]
pub fn chain_greeks(w: &ChainWireView) -> Option<dataplane::options::Greeks> {
    let s: f64 = w.underlying_price.as_deref()?.parse().ok()?;
    let k: f64 = w.strike.parse().ok()?;
    let expiry = chrono::NaiveDate::parse_from_str(&w.expiry, "%Y-%m-%d").ok()?;
    let expiry_t = expiry.and_hms_opt(20, 0, 0)?.and_utc();
    let event = DateTime::from_timestamp_nanos(w.event_ns);
    let t = (expiry_t - event).num_seconds() as f64 / (365.25 * 86_400.0);
    let right = match w.right.as_str() {
        "C" => dataplane::options::Right::Call,
        "P" => dataplane::options::Right::Put,
        _ => return None,
    };
    dataplane::options::bsm_greeks(right, s, k, t, w.iv_rate?, w.iv_div?, w.iv?)
}

/// The fields `chain_greeks` reads, so it can be tested without a ClickHouse row.
pub struct ChainWireView {
    pub underlying_price: Option<String>,
    pub strike: String,
    pub expiry: String,
    pub right: String,
    pub event_ns: i64,
    pub iv: Option<f64>,
    pub iv_rate: Option<f64>,
    pub iv_div: Option<f64>,
}

/// GET /api/data/option_chain — the chain as of `as_of`, IV stored and greeks
/// computed on the way out (DATA-005 §5).
pub async fn get_option_chain(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<OptionChainQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (as_of, cutoff_applied) = project.clip(q.as_of.unwrap_or_else(Utc::now));
    let manifest = json!({
        "type": "option_chain",
        "underlying": q.underlying,
        "venue": q.venue,
        "as_of": as_of,
        "cutoff_applied": cutoff_applied,
        "window": {"start": as_of, "end": as_of},
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
        "greeks": "black_scholes_merton, computed at read time from stored iv, rate, dividend and underlying price",
    });
    let client = storage::clickhouse::connect(&state.clickhouse_url);
    let wire: Vec<ChainWire> = match client
        .query(&chain_sql(&q.underlying, q.venue.as_deref(), as_of))
        .fetch_all()
        .await
    {
        Ok(w) => w,
        Err(e) if is_missing_table(&e.to_string()) => {
            return (
                StatusCode::OK,
                Json(unmeasured_body(
                    manifest,
                    "contracts",
                    "option_chain_snapshots",
                )),
            )
                .into_response()
        }
        Err(e) => return query_failed(e),
    };
    let rows: Vec<serde_json::Value> = wire
        .into_iter()
        .map(|w| {
            let g = chain_greeks(&ChainWireView {
                underlying_price: w.underlying_s.clone(),
                strike: w.strike_s.clone(),
                expiry: w.expiry_s.clone(),
                right: w.opt_right.clone(),
                event_ns: w.event_ns,
                iv: w.iv,
                iv_rate: w.iv_rate,
                iv_div: w.iv_div,
            });
            json!({
                "instrument": w.instrument, "expiry": w.expiry_s, "strike": w.strike_s, "right": w.opt_right,
                "event_time_ns": w.event_ns, "knowledge_time_ns": w.knowledge_ns,
                "bid": w.bid_s, "ask": w.ask_s, "mark": w.mark_s, "underlying_price": w.underlying_s,
                "volume": w.volume, "open_interest": w.open_interest,
                "iv": w.iv, "iv_model": w.iv_model,
                "in_liquid_universe": w.in_liquid_universe == 1,
                "delta": g.map(|g| g.delta), "gamma": g.map(|g| g.gamma), "vega": g.map(|g| g.vega),
                "theta": g.map(|g| g.theta), "rho": g.map(|g| g.rho),
            })
        })
        .collect();
    log_data_read(
        &state,
        &project,
        token.user_id(),
        vec![q.underlying.clone()],
        (as_of, as_of),
        &["bid", "ask", "iv", "open_interest"],
        format!("option_chain {} as of {as_of}", q.underlying),
    )
    .await;
    deliver(
        &state,
        &project,
        "option_chain",
        json!({"underlying": q.underlying, "venue": q.venue}),
        as_of,
        manifest,
        "contracts",
        rows,
        CHAIN_COLS,
        q.format.as_deref(),
    )
    .await
}

// ── fundamentals ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FundamentalsQuery {
    /// SEC CIK. A ticker cannot be resolved here: the platform keeps no
    /// ticker-to-CIK map, and guessing one would attach a filer's numbers to the
    /// wrong company. `ticker` is accepted only to be refused with that reason.
    #[serde(default)]
    pub cik: Option<i64>,
    #[serde(default)]
    pub ticker: Option<String>,
    /// Comma-separated XBRL tags, e.g. `Revenues,NetIncomeLoss`.
    #[serde(default)]
    pub fields: Option<String>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

const FUNDAMENTAL_COLS: &[(&str, PqCol)] = &[
    ("tag", PqCol::Str),
    ("unit", PqCol::Str),
    ("period_start", PqCol::Str),
    ("period_end", PqCol::Str),
    ("form", PqCol::Str),
    ("accession", PqCol::Str),
    ("value", PqCol::Dec),
    ("accepted_at_ns", PqCol::TsNs),
    ("knowledge", PqCol::Str),
];

/// GET /api/data/fundamentals — EDGAR XBRL facts with `accepted_at <= as_of`.
///
/// Per `(tag, unit, period_end)` the value from the latest filing accepted by
/// `as_of`: a restatement filed after `as_of` is invisible, and the period a
/// number describes is never mistaken for when it became knowable.
pub async fn get_fundamentals(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<FundamentalsQuery>,
) -> impl IntoResponse {
    use sqlx::Row;
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let Some(cik) = q.cik else {
        return bad_request(
            "cik_required",
            if q.ticker.is_some() {
                "a ticker cannot be resolved to a CIK on this server (no ticker map is kept, and a wrong guess would attach another company's numbers); pass `cik`".into()
            } else {
                "pass `cik`".into()
            },
        );
    };
    let (as_of, cutoff_applied) = project.clip(q.as_of.unwrap_or_else(Utc::now));
    let tags: Option<Vec<String>> = q.fields.as_deref().map(|f| {
        f.split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect()
    });
    let rows = match sqlx::query(
        "SELECT DISTINCT ON (tag, unit, period_end) \
                tag, unit, period_start::text AS period_start, period_end::text AS period_end, \
                form, accession, value::text AS value, accepted_at, knowledge \
           FROM dataplane.fundamentals_fact \
          WHERE cik = $1 AND accepted_at <= $2 AND ($3::text[] IS NULL OR tag = ANY($3)) \
          ORDER BY tag, unit, period_end, accepted_at DESC, accession DESC \
          LIMIT 50000",
    )
    .bind(cik)
    .bind(as_of)
    .bind(&tags)
    .fetch_all(&state.pg)
    .await
    {
        Ok(r) => r,
        Err(e) => return query_failed(e),
    };
    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let accepted: DateTime<Utc> = r.get("accepted_at");
            json!({
                "tag": r.get::<String, _>("tag"),
                "unit": r.get::<String, _>("unit"),
                "period_start": r.get::<Option<String>, _>("period_start"),
                "period_end": r.get::<String, _>("period_end"),
                "form": r.get::<String, _>("form"),
                "accession": r.get::<String, _>("accession"),
                "value": r.get::<String, _>("value"),
                "accepted_at_ns": accepted.timestamp_nanos_opt(),
                "knowledge": r.get::<String, _>("knowledge"),
            })
        })
        .collect();
    let manifest = json!({
        "type": "fundamentals",
        "cik": cik,
        "fields": tags,
        "as_of": as_of,
        "cutoff_applied": cutoff_applied,
        "window": {"start": as_of, "end": as_of},
        "revision_policy": "latest_filing_accepted_by_as_of",
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
    });
    log_data_read(
        &state,
        &project,
        token.user_id(),
        vec![format!("cik:{cik}")],
        (as_of, as_of),
        &["fundamentals"],
        format!("fundamentals cik {cik} as of {as_of}"),
    )
    .await;
    deliver(
        &state,
        &project,
        "fundamentals",
        json!({"cik": cik, "fields": tags}),
        as_of,
        manifest,
        "facts",
        out,
        FUNDAMENTAL_COLS,
        q.format.as_deref(),
    )
    .await
}

// ── search ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub asset_class: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// One searchable instrument: the best-covered timeframe's span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCandidate {
    pub instrument_id: String,
    pub asset_class: Option<String>,
    pub bars: u64,
    pub last_ns: i64,
}

/// Rank candidates against a query: exact match, then prefix, then substring;
/// within a tier the better-covered instrument first (coverage is the liquidity
/// proxy this store has), then the name for a stable order.
#[must_use]
pub fn rank_search(
    query: &str,
    asset_class: Option<&str>,
    mut candidates: Vec<SearchCandidate>,
    limit: usize,
) -> Vec<SearchCandidate> {
    let q = query.trim().to_ascii_lowercase();
    let tier = |id: &str| -> Option<u8> {
        let id = id.to_ascii_lowercase();
        if id == q {
            Some(0)
        } else if id.starts_with(&q) {
            Some(1)
        } else if id.contains(&q) {
            Some(2)
        } else {
            None
        }
    };
    candidates.retain(|c| {
        tier(&c.instrument_id).is_some()
            && asset_class.is_none_or(|a| c.asset_class.as_deref() == Some(a))
    });
    candidates.sort_by(|a, b| {
        tier(&a.instrument_id)
            .cmp(&tier(&b.instrument_id))
            .then(b.bars.cmp(&a.bars))
            .then(a.instrument_id.cmp(&b.instrument_id))
    });
    candidates.truncate(limit);
    candidates
}

/// GET /api/data/search — ranked instrument search over what the project can see.
pub async fn get_search(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    if q.q.trim().is_empty() {
        return bad_request("empty_query", "pass a non-empty `q`".into());
    }
    // Coverage through the project's horizon, as `get_catalog` does: a search that
    // surfaced instruments whose history starts after the cutoff would leak it.
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(project.horizon());
    let coverage = match store.list_coverage().await {
        Ok(c) => c,
        Err(e) => return query_failed(e),
    };
    let classes: std::collections::HashMap<String, String> =
        match storage::postgres::instruments::list_active(&state.pg).await {
            Ok(list) => list
                .into_iter()
                .filter_map(|i| {
                    serde_json::to_value(i.asset_class)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .map(|c| (i.instrument_id, c))
                })
                .collect(),
            Err(_) => std::collections::HashMap::new(),
        };
    let mut best: std::collections::HashMap<String, SearchCandidate> =
        std::collections::HashMap::new();
    for c in coverage {
        let e = best
            .entry(c.instrument_id.clone())
            .or_insert_with(|| SearchCandidate {
                instrument_id: c.instrument_id.clone(),
                asset_class: classes.get(&c.instrument_id).cloned(),
                bars: 0,
                last_ns: 0,
            });
        e.bars = e.bars.max(c.bars);
        e.last_ns = e.last_ns.max(c.last_ns);
    }
    let ranked = rank_search(
        &q.q,
        q.asset_class.as_deref(),
        best.into_values().collect(),
        q.limit.unwrap_or(25).clamp(1, 200),
    );
    (
        StatusCode::OK,
        Json(json!({
            "manifest": {
                "type": "search",
                "q": q.q,
                "asset_class": q.asset_class,
                "horizon": project.horizon(),
                "project_id": project.project_id,
                "rows": ranked.len(),
                "ranking": "exact, prefix, substring; then bars covered (the liquidity proxy this store has)",
            },
            "instruments": ranked.iter().map(|c| json!({
                "instrument_id": c.instrument_id,
                "asset_class": c.asset_class,
                "bars": c.bars,
                "last_ns": c.last_ns,
            })).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

// ── features (batch) ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FeaturesQuery {
    /// Comma-separated feature names, resolved through the one feature runtime.
    pub name: String,
    pub instrument: String,
    #[serde(default = "default_timeframe")]
    pub tf: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    /// Bars read before `start` so windowed features are warm at `start`.
    #[serde(default)]
    pub warmup: Option<u32>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// One requested feature column: a windowed feature computed from bars, or an
/// external one (P-3.6) computed from handed-in point-in-time rows.
enum FeatureColumn {
    Windowed(Arc<dyn features::Feature>),
    External(Arc<dyn features::external_features::ExternalFeature>),
}

/// GET /api/data/features — feature columns over a window (DATA-005 §5, DATA-006
/// batch mode). Scope `research:features`.
///
/// Computed by the same `features::runtime` the dataset builder, the backtest and
/// the live server use (INV-13/14), over bars read point-in-time and densified onto
/// the UTC master clock. A value whose window is not yet full is `null`, never a
/// fabricated zero.
pub async fn get_features(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<FeaturesQuery>,
) -> impl IntoResponse {
    if !crate::auth::scopes::permits(&token.scopes, crate::auth::scopes::FEATURES) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "scope_required",
                "message": format!("features need the {} scope", crate::auth::scopes::FEATURES),
            })),
        )
            .into_response();
    }
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    let (effective_end, effective_as_of, cutoff_applied) =
        match clip_window(&project, q.start, q.end, q.as_of) {
            Ok(w) => w,
            Err(resp) => return resp,
        };
    let Some(step_secs) = tf_secs(&q.tf) else {
        return bad_request("unknown_timeframe", format!("unknown timeframe {:?}", q.tf));
    };
    let names: Vec<String> = q
        .name
        .split(',')
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() || names.len() > 32 {
        return bad_request(
            "bad_feature_list",
            "give 1 to 32 comma-separated feature names".into(),
        );
    }
    // A name that declares an external input (`macro_<series>`, `fundamental_<cik>_<tag>`,
    // P-3.6) is not a windowed feature: it resolves against point-in-time rows handed
    // in below, never against bars alone.
    let mut feats: Vec<(String, FeatureColumn)> = Vec::new();
    for n in &names {
        match features::external_features::resolve_external(n, step_secs) {
            Ok(Some(f)) => {
                feats.push((n.clone(), FeatureColumn::External(f)));
                continue;
            }
            Ok(None) => {}
            Err(e) => return bad_request("unknown_feature", e.to_string()),
        }
        match features::runtime::feature(n) {
            Ok(f) => feats.push((n.clone(), FeatureColumn::Windowed(f))),
            Err(e) => return bad_request("unknown_feature", e.to_string()),
        }
    }
    let Some(timeframe) =
        <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(&q.tf)
    else {
        return bad_request("unknown_timeframe", format!("unknown timeframe {:?}", q.tf));
    };
    let warmup = i64::from(q.warmup.unwrap_or(500).min(5_000));
    let read_from = q.start - chrono::Duration::seconds(warmup * i64::from(step_secs));
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(effective_as_of);
    let bars = match store
        .load_bars(&q.instrument, timeframe, read_from, effective_end)
        .await
    {
        Ok(b) => b,
        Err(e) => return query_failed(e),
    };
    let clock = features::densify_bars(
        &bars
            .iter()
            .map(backtest::warmup::bar_obs)
            .collect::<Vec<_>>(),
        i64::from(step_secs) * 1_000_000_000,
    );
    let start_ns = q.start.timestamp_nanos_opt().unwrap_or(i64::MIN);
    let external: Vec<Arc<dyn features::external_features::ExternalFeature>> = feats
        .iter()
        .filter_map(|(_, f)| match f {
            FeatureColumn::External(e) => Some(Arc::clone(e)),
            FeatureColumn::Windowed(_) => None,
        })
        .collect();
    let external_rows = if external.is_empty() {
        crate::features_compute::ExternalRows::default()
    } else {
        match crate::features_compute::load_external_rows(
            &state.pg,
            &state.clickhouse_url,
            &external,
            read_from,
            effective_end,
            effective_as_of,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return query_failed(e),
        }
    };
    let columns: Vec<Vec<Option<f64>>> = feats
        .iter()
        .map(|(_, f)| match f {
            FeatureColumn::Windowed(f) => {
                features::runtime::backfill_column(f.as_ref(), &clock.rows)
            }
            FeatureColumn::External(f) => features::external_features::backfill_column_external(
                f.as_ref(),
                &clock.rows,
                &external_rows.inputs(),
            ),
        })
        .collect();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for (i, row) in clock.rows.iter().enumerate() {
        if row.ts_ns < start_ns {
            continue;
        }
        let mut r = serde_json::Map::new();
        r.insert("ts_ns".into(), json!(row.ts_ns));
        for (j, (name, _)) in feats.iter().enumerate() {
            let [value_col, age_col, quality_col] = features::align::companion_columns(name);
            r.insert(value_col, json!(columns[j][i]));
            r.insert(age_col, json!(clock.age_minutes.get(i)));
            r.insert(quality_col, json!(clock.quality.get(i).map(|f| f.0)));
        }
        rows.push(serde_json::Value::Object(r));
    }
    let mut cols: Vec<(String, PqCol)> = vec![("ts_ns".into(), PqCol::TsNs)];
    for n in &names {
        let [v, a, qf] = features::align::companion_columns(n);
        cols.push((v, PqCol::F64));
        cols.push((a, PqCol::I64));
        cols.push((qf, PqCol::I64));
    }
    let cols_ref: Vec<(&str, PqCol)> = cols.iter().map(|(n, k)| (n.as_str(), *k)).collect();
    let manifest = json!({
        "type": "features",
        "instrument": q.instrument,
        "timeframe": q.tf,
        "features": names,
        "warmup_bars": warmup,
        "window": {"start": q.start, "end": effective_end},
        "as_of": effective_as_of,
        "cutoff_applied": cutoff_applied,
        "project_id": project.project_id,
        "project_kind": project.kind.as_str(),
        "null_means": "the feature's window is not yet full, or the value is not finite; never zero",
    });
    log_data_read(
        &state,
        &project,
        token.user_id(),
        vec![q.instrument.clone()],
        (q.start, effective_end),
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
        format!(
            "features {} {} {:?} [{}, {})",
            q.instrument, q.tf, names, q.start, effective_end
        ),
    )
    .await;
    deliver(
        &state,
        &project,
        "features",
        json!({"instrument": q.instrument, "tf": q.tf, "names": names, "start": q.start, "end": effective_end, "warmup": warmup}),
        effective_as_of,
        manifest,
        "features",
        rows,
        &cols_ref,
        q.format.as_deref(),
    )
    .await
}

#[cfg(test)]
mod data_v2_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_snapshot_id_pins_the_data_and_moves_when_the_data_does() {
        let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let p = json!({"instrument": "BTC-USD", "tf": "1m"});
        let a = data_snapshot_id("bars", &p, as_of, "aaaa");
        assert_eq!(a, data_snapshot_id("bars", &p, as_of, "aaaa"));
        assert!(a.starts_with("ds_") && a.len() == 35);
        // A restated row, a different window, a different as_of and a different
        // endpoint each move the id.
        assert_ne!(a, data_snapshot_id("bars", &p, as_of, "bbbb"));
        assert_ne!(
            a,
            data_snapshot_id(
                "bars",
                &json!({"instrument": "ETH-USD", "tf": "1m"}),
                as_of,
                "aaaa"
            )
        );
        assert_ne!(
            a,
            data_snapshot_id("bars", &p, as_of + chrono::Duration::seconds(1), "aaaa")
        );
        assert_ne!(a, data_snapshot_id("trades", &p, as_of, "aaaa"));
        // Key order in the params does not.
        let reordered = json!({"tf": "1m", "instrument": "BTC-USD"});
        assert_eq!(a, data_snapshot_id("bars", &reordered, as_of, "aaaa"));
    }

    #[test]
    fn parquet_round_trips_decimals_exactly_and_keeps_nulls_null() {
        use arrow::array::{Array, Decimal128Array};
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let rows = vec![
            json!({"ts_ns": 1_700_000_000_000_000_000_i64, "price": "50000.1234567891", "n": 3, "x": 1.5, "s": "buy"}),
            json!({"ts_ns": 1_700_000_060_000_000_000_i64, "price": null, "n": null, "x": null, "s": null}),
        ];
        let bytes = rows_to_parquet(
            &[
                ("ts_ns", PqCol::TsNs),
                ("price", PqCol::Dec),
                ("n", PqCol::I64),
                ("x", PqCol::F64),
                ("s", PqCol::Str),
            ],
            &rows,
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!("data_v2_{}.parquet", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let batch = reader.into_iter().next().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        let price = batch
            .column_by_name("price")
            .unwrap()
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(price.precision(), 38);
        assert_eq!(price.scale(), 10);
        assert_eq!(price.value(0), 500_001_234_567_891_i128);
        assert!(price.is_null(1), "an absent price is null, not zero");
        assert!(batch.column_by_name("n").unwrap().is_null(1));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_decimal_beyond_ten_places_is_rounded_not_rejected() {
        assert_eq!(decimal_to_i128_scale10("1").unwrap(), 10_000_000_000);
        assert_eq!(decimal_to_i128_scale10("0.00000000006").unwrap(), 1);
        // Half-way cases round to even (rust_decimal's default), not up.
        assert_eq!(decimal_to_i128_scale10("0.00000000005").unwrap(), 0);
        assert!(decimal_to_i128_scale10("abc").is_err());
    }

    #[test]
    fn the_trades_query_is_point_in_time_and_deduplicated() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let sql = trades_sql("BTC-USD", t, t + chrono::Duration::hours(1), t, 10);
        assert!(sql.contains("available_time <="), "{sql}");
        assert!(sql.contains("LIMIT 1 BY dedup_key"), "{sql}");
        assert!(
            sql.contains("LIMIT 11"),
            "one extra row detects truncation: {sql}"
        );
        let evil = trades_sql("x' OR '1'='1", t, t, t, 1);
        assert!(evil.contains("x\\' OR \\'1\\'=\\'1"), "{evil}");
    }

    #[test]
    fn the_chain_query_reads_one_snapshot_known_by_as_of() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let sql = chain_sql("SPY", Some("tradier"), t);
        assert_eq!(sql.matches("knowledge_time <=").count(), 2, "{sql}");
        assert!(sql.contains("max(event_time)"), "{sql}");
        assert!(sql.contains("venue = 'tradier'"), "{sql}");
    }

    #[test]
    fn greeks_come_from_stored_inputs_and_a_missing_iv_is_not_a_zero() {
        let event = Utc.with_ymd_and_hms(2026, 6, 1, 15, 0, 0).unwrap();
        let mut w = ChainWireView {
            underlying_price: Some("100".into()),
            strike: "100".into(),
            expiry: "2026-07-01".into(),
            right: "C".into(),
            event_ns: event.timestamp_nanos_opt().unwrap(),
            iv: Some(0.2),
            iv_rate: Some(0.04),
            iv_div: Some(0.0),
        };
        let g = chain_greeks(&w).expect("inputs are complete");
        assert!(
            g.delta > 0.4 && g.delta < 0.7,
            "an at-the-money call's delta: {}",
            g.delta
        );
        assert!(g.gamma > 0.0 && g.vega > 0.0);
        w.iv = None;
        assert!(chain_greeks(&w).is_none());
        w.iv = Some(0.2);
        w.right = "P".into();
        assert!(chain_greeks(&w).unwrap().delta < 0.0);
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_substring_then_coverage() {
        let c = |id: &str, class: &str, bars: u64| SearchCandidate {
            instrument_id: id.into(),
            asset_class: Some(class.into()),
            bars,
            last_ns: 0,
        };
        let ranked = rank_search(
            "btc",
            None,
            vec![
                c("WBTC-USD", "crypto_spot_cex", 9_000),
                c("BTC-USD", "crypto_spot_cex", 10),
                c("BTCUSDT-PERP", "crypto_perp", 500),
                c("BTC-EUR", "crypto_spot_cex", 5_000),
                c("ETH-USD", "crypto_spot_cex", 99_999),
            ],
            10,
        );
        let ids: Vec<&str> = ranked.iter().map(|r| r.instrument_id.as_str()).collect();
        assert_eq!(ids, ["BTC-EUR", "BTCUSDT-PERP", "BTC-USD", "WBTC-USD"]);
        let only_perp = rank_search("btc", Some("crypto_perp"), ranked, 10);
        assert_eq!(only_perp.len(), 1);
    }

    #[test]
    fn a_table_that_does_not_exist_is_unmeasured_not_empty() {
        assert!(is_missing_table(
            "Code: 60. DB::Exception: Table trading.market_trades doesn't exist"
        ));
        assert!(!is_missing_table("Code: 62. Syntax error"));
        let body = unmeasured_body(json!({"type": "trades"}), "trades", "market_trades");
        assert!(body["trades"].is_null());
        assert_eq!(body["manifest"]["measured"], false);
    }
}

use std::sync::atomic::{AtomicBool, AtomicU64};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use backtest::{
    collect::{collect_ranges, CollectorPlan},
    BarStore, MissingRange,
};
use chrono::{Duration, Utc};
use domain::payloads::bar::Timeframe;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

// ── GET /assets/initialized ───────────────────────────────────────────────────

pub async fn list_initialized(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT symbol, asset_class FROM asset_lifecycle ORDER BY symbol")
            .fetch_all(&state.pg)
            .await
            .unwrap_or_default();
    let assets: Vec<serde_json::Value> = rows
        .iter()
        .map(|(sym, ac)| json!({ "symbol": sym, "asset_class": ac }))
        .collect();
    Json(json!({ "assets": assets }))
}

// ── GET /assets/lifecycle/:symbol ─────────────────────────────────────────────

pub async fn get_lifecycle(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    let row: Option<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT state, asset_class, venue_id, strategy_id, execution_mode \
         FROM asset_lifecycle WHERE symbol = $1",
    )
    .bind(&symbol)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    match row {
        Some((lc_state, asset_class, venue_id, strategy_id, exec_mode)) => Json(json!({
            "symbol": symbol,
            "lifecycle": lc_state,
            "asset_class": asset_class,
            "venue_id": venue_id,
            "strategy_id": strategy_id,
            "execution_mode": exec_mode,
        }))
        .into_response(),
        None => Json(json!({ "symbol": symbol, "lifecycle": "uninitialized" })).into_response(),
    }
}

// ── POST /assets/init/:symbol ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct InitBody {
    pub lookback_days: u32,
    pub asset_class: Option<String>,
}

pub async fn init_asset(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<InitBody>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let err = |msg: &str| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": msg })),
        )
    };

    let lookback_days = body.lookback_days.clamp(1, 3650);

    // Resolve asset_class: explicit > instruments table > heuristic
    let asset_class = if let Some(ac) = body.asset_class.filter(|s| !s.is_empty()) {
        ac
    } else {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT asset_class FROM instruments WHERE instrument_id = $1")
                .bind(&symbol)
                .fetch_optional(&state.pg)
                .await
                .map_err(|_| err("db error"))?;
        row.map(|(ac,)| ac)
            .unwrap_or_else(|| infer_asset_class(&symbol))
    };

    let venue_id = venue_for_asset_class(&asset_class);

    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO asset_init_jobs (symbol, lookback_days) VALUES ($1, $2) RETURNING job_id",
    )
    .bind(&symbol)
    .bind(lookback_days as i32)
    .fetch_one(&state.pg)
    .await
    .map_err(|_| err("failed to create job"))?;

    // Spawn background bar-seeding task
    let pg = state.pg.clone();
    let ch_url = state.clickhouse_url.clone();
    let sym = symbol.clone();
    let ac = asset_class.clone();
    let vid = venue_id.clone();
    let stream_tx = state.stream_tx.clone();

    tokio::spawn(async move {
        let result = seed_bars(&ch_url, &sym, &ac, &vid, lookback_days, &pg, job_id).await;

        match result {
            Ok(bars) => {
                let _ = sqlx::query(
                    "UPDATE asset_init_jobs \
                     SET status = 'done', bars_collected = $1, finished_at = now() \
                     WHERE job_id = $2",
                )
                .bind(bars as i64)
                .bind(job_id)
                .execute(&pg)
                .await;

                let _ = sqlx::query(
                    "INSERT INTO asset_lifecycle (symbol, asset_class, venue_id, state) \
                     VALUES ($1, $2, $3, 'initialized_not_active') \
                     ON CONFLICT (symbol) DO UPDATE \
                     SET state = 'initialized_not_active', updated_at = now()",
                )
                .bind(&sym)
                .bind(&ac)
                .bind(&vid)
                .execute(&pg)
                .await;

                // Start continuous live 1-minute aggregation for this asset now,
                // so it keeps accumulating minute bars without a restart.
                if let Some(tx) = &stream_tx {
                    let _ = tx.send(crate::state::StreamRequest {
                        instrument_id: sym.clone(),
                        asset_class: ac.clone(),
                    });
                }

                tracing::info!(symbol = %sym, bars, "asset init complete");
            }
            Err(e) => {
                tracing::error!(symbol = %sym, error = %e, "asset init failed");
                let _ = sqlx::query(
                    "UPDATE asset_init_jobs \
                     SET status = 'error', error = $1, finished_at = now() \
                     WHERE job_id = $2",
                )
                .bind(e.to_string())
                .bind(job_id)
                .execute(&pg)
                .await;
            }
        }
    });

    Ok(Json(json!({
        "job_id": job_id.to_string(),
        "symbol": symbol,
        "asset_class": asset_class,
        "lookback_days": lookback_days,
    })))
}

async fn seed_bars(
    ch_url: &str,
    symbol: &str,
    asset_class: &str,
    venue_id: &str,
    lookback_days: u32,
    pg: &sqlx::PgPool,
    job_id: Uuid,
) -> anyhow::Result<u64> {
    let plan = CollectorPlan::for_asset_class(asset_class, symbol)?;
    let store = BarStore::connect(ch_url);
    let http = reqwest::Client::new();

    let now = Utc::now();
    let from = now - Duration::days(i64::from(lookback_days));
    let range = MissingRange { from, to: now };

    let collected = AtomicU64::new(0);
    let cancel = AtomicBool::new(false);

    // Pass 1 — 1h bars for the long-term chart view.
    let h1_total = collect_ranges(
        &http,
        &store,
        &plan,
        symbol,
        venue_id,
        Timeframe::Hours1,
        &[range.clone()],
        &collected,
        &cancel,
    )
    .await?;

    let _ = sqlx::query("UPDATE asset_init_jobs SET bars_collected = $1 WHERE job_id = $2")
        .bind(collected.load(std::sync::atomic::Ordering::Relaxed) as i64)
        .bind(job_id)
        .execute(pg)
        .await;

    // Pass 2 — 1m bars so 5m/15m/30m views are immediately populated from
    // stored data rather than waiting for live accumulation.
    let m1_total = collect_ranges(
        &http,
        &store,
        &plan,
        symbol,
        venue_id,
        Timeframe::Minutes1,
        &[range],
        &collected,
        &cancel,
    )
    .await?;

    let _ = sqlx::query("UPDATE asset_init_jobs SET bars_collected = $1 WHERE job_id = $2")
        .bind(collected.load(std::sync::atomic::Ordering::Relaxed) as i64)
        .bind(job_id)
        .execute(pg)
        .await;

    Ok(h1_total + m1_total)
}

// ── GET /assets/init/jobs/:job_id ─────────────────────────────────────────────

pub async fn get_init_job(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> impl IntoResponse {
    let row: Option<(String, String, i32, i64, Option<String>)> = sqlx::query_as(
        "SELECT symbol, status, lookback_days, bars_collected, error \
         FROM asset_init_jobs WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    match row {
        Some((symbol, status, lookback_days, bars_collected, error)) => Json(json!({
            "job_id": job_id,
            "symbol": symbol,
            "status": status,
            "lookback_days": lookback_days,
            "bars_collected": bars_collected,
            "error": error,
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "job not found" })),
        )
            .into_response(),
    }
}

// ── POST /assets/lifecycle/:symbol/start ──────────────────────────────────────

pub async fn start_asset(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "UPDATE asset_lifecycle SET state = 'active', updated_at = now() WHERE symbol = $1",
    )
    .bind(&symbol)
    .execute(&state.pg)
    .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            Json(json!({ "symbol": symbol, "state": "active" })).into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "asset not initialized" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "db error" })),
        )
            .into_response(),
    }
}

// ── POST /assets/lifecycle/:symbol/stop ───────────────────────────────────────

pub async fn stop_asset(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "UPDATE asset_lifecycle \
         SET state = 'initialized_not_active', updated_at = now() \
         WHERE symbol = $1",
    )
    .bind(&symbol)
    .execute(&state.pg)
    .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            Json(json!({ "symbol": symbol, "state": "initialized_not_active" })).into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "asset not found" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "db error" })),
        )
            .into_response(),
    }
}

// ── GET /assets/strategy/:symbol ──────────────────────────────────────────────

pub async fn get_asset_strategy(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT strategy_id FROM asset_lifecycle WHERE symbol = $1")
            .bind(&symbol)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

    Json(json!({
        "symbol": symbol,
        "strategy_id": row.and_then(|(id,)| id),
    }))
}

// ── PUT /assets/strategy/:symbol ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StrategyBody {
    pub strategy_id: String,
}

pub async fn set_asset_strategy(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<StrategyBody>,
) -> impl IntoResponse {
    match sqlx::query(
        "UPDATE asset_lifecycle SET strategy_id = $1, updated_at = now() WHERE symbol = $2",
    )
    .bind(&body.strategy_id)
    .bind(&symbol)
    .execute(&state.pg)
    .await
    {
        Ok(r) if r.rows_affected() > 0 => Json(json!({ "ok": true })).into_response(),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "asset not initialized" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "db error" })),
        )
            .into_response(),
    }
}

// ── DELETE /assets/strategy/:symbol ───────────────────────────────────────────

pub async fn delete_asset_strategy(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE asset_lifecycle SET strategy_id = NULL, updated_at = now() WHERE symbol = $1",
    )
    .bind(&symbol)
    .execute(&state.pg)
    .await;
    Json(json!({ "ok": true }))
}

// ── GET /assets/execution-mode/:symbol ────────────────────────────────────────

pub async fn get_exec_mode(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT execution_mode FROM asset_lifecycle WHERE symbol = $1")
            .bind(&symbol)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

    Json(json!({
        "symbol": symbol,
        "mode": row.map(|(m,)| m).unwrap_or_else(|| "paper".to_string()),
    }))
}

// ── PUT /assets/execution-mode/:symbol ────────────────────────────────────────

#[derive(Deserialize)]
pub struct ExecModeBody {
    pub mode: String,
}

pub async fn set_exec_mode(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<ExecModeBody>,
) -> impl IntoResponse {
    let mode = if body.mode == "live" { "live" } else { "paper" };
    match sqlx::query(
        "UPDATE asset_lifecycle SET execution_mode = $1, updated_at = now() WHERE symbol = $2",
    )
    .bind(mode)
    .bind(&symbol)
    .execute(&state.pg)
    .await
    {
        Ok(r) if r.rows_affected() > 0 => Json(json!({ "ok": true })).into_response(),
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "asset not initialized" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "db error" })),
        )
            .into_response(),
    }
}

// ── DELETE /assets/execution-mode/:symbol ─────────────────────────────────────

pub async fn delete_exec_mode(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE asset_lifecycle SET execution_mode = 'paper', updated_at = now() WHERE symbol = $1",
    )
    .bind(&symbol)
    .execute(&state.pg)
    .await;
    Json(json!({ "ok": true }))
}

// ── GET /assets/models/:symbol ────────────────────────────────────────────────

pub async fn get_models(_token: BearerToken, Path(symbol): Path<String>) -> impl IntoResponse {
    Json(json!({ "symbol": symbol }))
}

// ── GET /assets/chart/bars ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct BarsQuery {
    pub symbol: String,
    pub start: String,
    pub end: String,
    pub interval_seconds: Option<u32>,
}

pub async fn get_chart_bars(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<BarsQuery>,
) -> impl IntoResponse {
    // Ensure the live 1-minute pipeline is running so the graph receives fresh
    // bars as they close.  Resolves asset class from DB with heuristic fallback.
    state.ensure_pipeline_for_instrument(&q.symbol).await;

    let interval = q.interval_seconds.unwrap_or(3600);

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                    .map(|dt| dt.and_utc().fixed_offset())
            })
            .map(|dt| dt.with_timezone(&Utc))
    };

    let from = parse_dt(&q.start).unwrap_or_else(|_| Utc::now() - Duration::days(7));
    let to = parse_dt(&q.end).unwrap_or_else(|_| Utc::now());

    let store = BarStore::connect(&state.clickhouse_url);
    // 1m bars are stored natively; all coarser intervals are aggregated from
    // the stored 1m rows on read.  This keeps every timeframe consistent with
    // the same data source and eliminates gaps that can appear in the
    // separately-seeded Timeframe::Hours1 rows.
    let result = match interval {
        60 => {
            store
                .load_bars(&q.symbol, Timeframe::Minutes1, from, to)
                .await
        }
        secs => {
            store
                .load_bars_bucketed(&q.symbol, Timeframe::Minutes1, secs, from, to)
                .await
        }
    };
    match result {
        Ok(bars) => {
            let out: Vec<serde_json::Value> = bars
                .iter()
                .map(|b| {
                    json!({
                        "t": b.ts_ns / 1_000_000_000,
                        "o": b.open.to_string(),
                        "h": b.high.to_string(),
                        "l": b.low.to_string(),
                        "c": b.close.to_string(),
                        "v": b.volume.to_string(),
                    })
                })
                .collect();
            Json(json!({ "bars": out })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── GET /assets/chart/trade-markers ───────────────────────────────────────────

#[derive(Deserialize)]
pub struct MarkersQuery {
    pub symbol: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

pub async fn get_trade_markers(
    _token: BearerToken,
    Query(_q): Query<MarkersQuery>,
) -> impl IntoResponse {
    Json(json!({ "markers": [] }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn infer_asset_class(symbol: &str) -> String {
    let u = symbol.to_uppercase();
    if u.ends_with("-USD")
        || u.ends_with("-USDT")
        || u.ends_with("-USDC")
        || u.ends_with("-BTC")
        || u.ends_with("-ETH")
        || u.ends_with("USDT")
        || u.ends_with("USDC")
    {
        "crypto_spot_cex".to_string()
    } else {
        "equity".to_string()
    }
}

fn venue_for_asset_class(asset_class: &str) -> String {
    match asset_class {
        "crypto_spot_cex" | "crypto_spot_dex" | "perpetual_swap" => "coinbase".to_string(),
        _ => "alpaca".to_string(),
    }
}

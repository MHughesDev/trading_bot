//! Portfolio, execution, risk, alert and settings endpoints.
//!
//! These back the interface surfaces that shipped ahead of their data:
//!
//! | Route                          | Feeds                                    |
//! |--------------------------------|------------------------------------------|
//! | `GET  /api/portfolio/equity-curve` | Dashboard equity curve + hero sparkline |
//! | `GET  /api/portfolio/positions`    | Dashboard open positions, desk panel    |
//! | `GET  /api/execution/orders`       | Cross-venue working orders              |
//! | `GET  /api/execution/fills`        | Execution blotter                       |
//! | `DELETE /api/orders/{id}`          | Cancel a working order                  |
//! | `GET  /api/risk/summary`           | Dashboard risk & exposure panel         |
//! | `GET  /api/account/transactions`   | Account activity                        |
//! | `/api/alerts` (CRUD)               | Price alerts                            |
//! | `/api/settings`                    | Preferences that follow the user        |
//!
//! Everything except alerts and settings is computed from the paper engine's
//! own state, so none of it can drift from what the dashboard rollup reports —
//! they read the same snapshots.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Datelike, Duration, Utc};
use execution::paper::ALL_ASSET_CLASSES;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

// ── shared helpers ───────────────────────────────────────────────────────────

fn dec_str(d: Decimal) -> String {
    d.normalize().to_string()
}

/// Asset classes whose account currency is the USD base. Everything else is
/// reported but excluded from cross-class sums — adding ETH to USD without a
/// rate is how a dashboard starts lying.
fn is_base_currency(currency: &str) -> bool {
    matches!(currency, "USD" | "USDC" | "USDT")
}

// ── equity curve ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EquityCurveQuery {
    /// 1D | 1W | 1M | 3M | YTD | ALL
    #[serde(default = "default_range")]
    range: String,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_range() -> String {
    "1M".to_owned()
}
fn default_mode() -> String {
    "PAPER".to_owned()
}

fn range_start(range: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match range.to_uppercase().as_str() {
        "1D" => Some(now - Duration::days(1)),
        "1W" => Some(now - Duration::weeks(1)),
        "1M" => Some(now - Duration::days(30)),
        "3M" => Some(now - Duration::days(90)),
        "YTD" => chrono::NaiveDate::from_ymd_opt(now.date_naive().year(), 1, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|d| DateTime::from_naive_utc_and_offset(d, Utc)),
        _ => None, // ALL
    }
}

#[derive(Debug, Serialize)]
pub struct EquityPoint {
    /// Unix seconds — the chart's x axis.
    t: i64,
    equity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    deposit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EquityCurveResponse {
    points: Vec<EquityPoint>,
    currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_drawdown_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sharpe: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_day: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worst_day: Option<String>,
}

/// `GET /api/portfolio/equity-curve?range=1M&mode=PAPER`
///
/// Reads the snapshots the sampler writes, summed across base-currency asset
/// classes at each sampled instant. Returns an empty series (not an error) when
/// nothing has been recorded yet — the screen has a designed state for that and
/// an empty curve is a true statement about a new account.
pub async fn equity_curve(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<EquityCurveQuery>,
) -> impl IntoResponse {
    let mode = q.mode.to_lowercase();
    let since = range_start(&q.range, Utc::now());

    let rows = sqlx::query_as::<_, (DateTime<Utc>, Decimal, Decimal)>(
        r#"
        SELECT at, SUM(equity) AS equity, SUM(cash_flow) AS cash_flow
        FROM equity_snapshots
        WHERE account_mode = $1
          AND currency IN ('USD', 'USDC', 'USDT')
          AND ($2::timestamptz IS NULL OR at >= $2)
        GROUP BY at
        ORDER BY at ASC
        "#,
    )
    .bind(&mode)
    .bind(since)
    .fetch_all(&state.pg)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "equity_curve_query_failed", "message": e.to_string() })),
            )
                .into_response();
        }
    };

    let points: Vec<EquityPoint> = rows
        .iter()
        .map(|(at, equity, flow)| EquityPoint {
            t: at.timestamp(),
            equity: dec_str(*equity),
            deposit: (!flow.is_zero()).then(|| dec_str(*flow)),
        })
        .collect();

    // Headline statistics, computed over the returned window only.
    let values: Vec<f64> = rows.iter().filter_map(|(_, e, _)| e.to_f64()).collect();
    let (max_dd, sharpe, best, worst) = curve_stats(&values);

    Json(EquityCurveResponse {
        points,
        currency: "USD".to_owned(),
        max_drawdown_pct: max_dd,
        sharpe,
        best_day: best.map(|v| format!("{v:.2}")),
        worst_day: worst.map(|v| format!("{v:.2}")),
    })
    .into_response()
}

/// Max drawdown %, annualised Sharpe of the sampled returns, and the best and
/// worst single-step moves. `None` where there are too few points to mean
/// anything — an under-defined statistic is worse than a blank.
fn curve_stats(values: &[f64]) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    if values.len() < 3 {
        return (None, None, None, None);
    }
    let mut peak = values[0];
    let mut max_dd = 0.0_f64;
    let mut deltas: Vec<f64> = Vec::with_capacity(values.len());
    let mut rets: Vec<f64> = Vec::with_capacity(values.len());
    for w in values.windows(2) {
        let (a, b) = (w[0], w[1]);
        deltas.push(b - a);
        if a != 0.0 {
            rets.push((b - a) / a.abs());
        }
        if b > peak {
            peak = b;
        }
        if peak != 0.0 {
            let dd = (peak - b) / peak.abs() * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    let mean = rets.iter().sum::<f64>() / rets.len().max(1) as f64;
    let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rets.len().max(1) as f64;
    let sd = var.sqrt();
    // Samples are taken on a fixed interval; annualise against a 252-day year.
    let sharpe = (sd > 0.0).then(|| mean / sd * (252.0_f64).sqrt());

    let best = deltas.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let worst = deltas.iter().cloned().fold(f64::INFINITY, f64::min);

    (
        Some(max_dd),
        sharpe,
        best.is_finite().then_some(best),
        worst.is_finite().then_some(worst),
    )
}

// ── positions ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PositionRow {
    instrument_id: String,
    asset_class: String,
    currency: String,
    side: &'static str,
    size: String,
    entry: String,
    mark: Option<String>,
    unrealized: String,
    notional: String,
    return_pct: Option<f64>,
}

/// `GET /api/portfolio/positions` — every open position, across every class.
pub async fn positions(_token: BearerToken, State(state): State<AppState>) -> impl IntoResponse {
    let mut out: Vec<PositionRow> = Vec::new();
    for snap in state.paper_engine.snapshots() {
        for p in &snap.positions {
            if p.quantity.is_zero() {
                continue;
            }
            let cost = p.quantity.abs() * p.average_entry_price;
            let return_pct = (!cost.is_zero())
                .then(|| (p.unrealized_pnl / cost * Decimal::from(100)).to_f64())
                .flatten();
            out.push(PositionRow {
                instrument_id: p.instrument_id.clone(),
                asset_class: snap.asset_class.as_str().to_owned(),
                currency: snap.currency.to_owned(),
                side: if p.quantity.is_sign_negative() {
                    "short"
                } else {
                    "long"
                },
                size: dec_str(p.quantity.abs()),
                entry: dec_str(p.average_entry_price),
                mark: p.mark_price.map(dec_str),
                unrealized: dec_str(p.unrealized_pnl),
                notional: dec_str(p.notional),
                return_pct,
            });
        }
    }
    out.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    Json(json!({ "positions": out })).into_response()
}

// ── orders and fills ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecutionQuery {
    instrument: Option<String>,
    asset_class: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Every order the engine retains, newest first, without needing an instrument.
fn all_orders(state: &AppState) -> Vec<execution::paper::PaperOrderView> {
    let mut instruments: Vec<String> = state
        .paper_engine
        .open_orders(None)
        .into_iter()
        .map(|o| o.instrument_id)
        .collect();
    // `open_orders` only reports working orders; the engine's per-instrument
    // view is the only way to reach terminal ones, so gather the instrument set
    // from positions and marks too.
    for snap in state.paper_engine.snapshots() {
        for p in &snap.positions {
            instruments.push(p.instrument_id.clone());
        }
    }
    instruments.sort();
    instruments.dedup();

    let mut out: Vec<execution::paper::PaperOrderView> = instruments
        .iter()
        .flat_map(|i| state.paper_engine.orders_for_instrument(i))
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    out
}

/// `GET /api/execution/orders` — working orders across every instrument.
pub async fn list_orders(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<ExecutionQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(500);
    let orders: Vec<_> = all_orders(&state)
        .into_iter()
        .filter(|o| matches!(o.status.as_str(), "new" | "partially_filled"))
        .filter(|o| q.instrument.as_ref().is_none_or(|i| &o.instrument_id == i))
        .filter(|o| {
            q.asset_class
                .as_ref()
                .is_none_or(|c| &o.asset_class == c)
        })
        .take(limit)
        .collect();
    Json(json!({ "orders": orders })).into_response()
}

#[derive(Debug, Serialize)]
pub struct FillRow {
    id: String,
    ts: DateTime<Utc>,
    instrument_id: String,
    asset_class: String,
    side: String,
    qty: String,
    price: Option<String>,
    value: Option<String>,
    status: String,
}

/// `GET /api/execution/fills` — the execution blotter.
pub async fn list_fills(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<ExecutionQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(200);
    let fills: Vec<FillRow> = all_orders(&state)
        .into_iter()
        .filter(|o| matches!(o.status.as_str(), "filled" | "partially_filled"))
        .filter(|o| q.instrument.as_ref().is_none_or(|i| &o.instrument_id == i))
        .filter(|o| q.asset_class.as_ref().is_none_or(|c| &o.asset_class == c))
        .take(limit)
        .map(|o| FillRow {
            id: o.order_id.clone(),
            ts: o.updated_at,
            instrument_id: o.instrument_id.clone(),
            asset_class: o.asset_class.clone(),
            side: o.side.clone(),
            qty: dec_str(o.filled_qty),
            price: o.avg_fill_price.map(dec_str),
            value: o
                .avg_fill_price
                .map(|p| dec_str(p * o.filled_qty)),
            status: o.status.clone(),
        })
        .collect();
    Json(json!({ "fills": fills })).into_response()
}

/// `DELETE /api/orders/{id}` — cancel a working order.
pub async fn cancel_order(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> impl IntoResponse {
    match state.paper_engine.cancel(&order_id) {
        Ok(()) => Json(json!({ "ok": true, "order_id": order_id })).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "cancel_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── account transactions ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TransactionQuery {
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    symbol: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TransactionRow {
    ts: DateTime<Utc>,
    symbol: String,
    side: String,
    quantity: String,
    source: String,
    correlation_id: Option<String>,
    execution_mode: Option<String>,
    cash_delta: String,
    note: String,
}

/// `GET /api/account/transactions` — the account journal, all classes merged.
///
/// This used to be proxied to a Python service that no longer runs. The paper
/// engine keeps the same journal in memory, so it is served from there.
pub async fn transactions(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<TransactionQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(1000);
    let mut rows: Vec<TransactionRow> = Vec::new();

    for ac in ALL_ASSET_CLASSES {
        for e in state.paper_engine.transactions_since(ac, q.start) {
            if let Some(end) = q.end {
                if e.at > end {
                    continue;
                }
            }
            let symbol = e.instrument_id.clone().unwrap_or_default();
            if let Some(want) = &q.symbol {
                if !symbol.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            // The journal note carries "buy 1 @ 50000"; the leading verb is the
            // side, which is what the activity table renders as a badge.
            let side = e
                .note
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();
            let quantity = e
                .note
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_owned();

            rows.push(TransactionRow {
                ts: e.at,
                symbol,
                side,
                quantity,
                source: format!("{:?}", e.kind).to_lowercase(),
                correlation_id: e.order_id.clone(),
                execution_mode: Some("paper".to_owned()),
                cash_delta: dec_str(e.cash_delta),
                note: e.note.clone(),
            });
        }
    }

    rows.sort_by_key(|r| std::cmp::Reverse(r.ts));
    rows.truncate(limit);
    Json(json!({ "transactions": rows })).into_response()
}

// ── risk summary ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RiskSummary {
    gross_exposure: String,
    net_exposure: String,
    net_exposure_pct_of_equity: f64,
    var95_one_day: Option<String>,
    margin_used_pct: f64,
    margin_limit_pct: f64,
    concentration_top_pct: Option<f64>,
    within_limits: bool,
    breaches: Vec<RiskBreach>,
    kill_switch_tripped: bool,
}

#[derive(Debug, Serialize)]
pub struct RiskBreach {
    label: String,
    detail: String,
}

/// `GET /api/risk/summary` — exposure and limit state for the dashboard panel.
///
/// Derived from the same snapshots the rollup uses, so the two can never
/// disagree. The margin limit is the platform default until the risk service
/// reads a per-user value.
pub async fn risk_summary(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    const MARGIN_LIMIT_PCT: f64 = 60.0;

    let snaps = state.paper_engine.snapshots();
    let mut equity = Decimal::ZERO;
    let mut used_margin = Decimal::ZERO;
    let mut gross = Decimal::ZERO;
    let mut net = Decimal::ZERO;
    let mut largest = Decimal::ZERO;

    for s in &snaps {
        if !is_base_currency(s.currency) {
            continue;
        }
        equity += s.equity;
        used_margin += s.used_margin;
        for p in &s.positions {
            let signed = if p.quantity.is_sign_negative() {
                -p.notional
            } else {
                p.notional
            };
            gross += p.notional.abs();
            net += signed;
            if p.notional.abs() > largest {
                largest = p.notional.abs();
            }
        }
    }

    let equity_f = equity.to_f64().unwrap_or(0.0);
    let net_pct = if equity_f != 0.0 {
        net.to_f64().unwrap_or(0.0) / equity_f * 100.0
    } else {
        0.0
    };
    let margin_pct = if equity_f != 0.0 {
        used_margin.to_f64().unwrap_or(0.0) / equity_f * 100.0
    } else {
        0.0
    };
    let concentration = (gross > Decimal::ZERO)
        .then(|| (largest / gross * Decimal::from(100)).to_f64())
        .flatten();

    let mut breaches = Vec::new();
    if margin_pct > MARGIN_LIMIT_PCT {
        breaches.push(RiskBreach {
            label: "Margin utilisation".to_owned(),
            detail: format!("{margin_pct:.1}% of equity is committed as margin, above the {MARGIN_LIMIT_PCT:.0}% limit."),
        });
    }
    if let Some(c) = concentration {
        if c > 60.0 {
            breaches.push(RiskBreach {
                label: "Concentration".to_owned(),
                detail: format!("{c:.0}% of gross exposure sits in a single position."),
            });
        }
    }

    // `is_active()` means the switch is TRIPPED, i.e. trading is halted.
    let tripped = state.kill_switch.is_active();
    if tripped {
        breaches.push(RiskBreach {
            label: "Kill switch".to_owned(),
            detail: "Order flow is halted platform-wide until it is reset.".to_owned(),
        });
    }

    Json(RiskSummary {
        gross_exposure: dec_str(gross),
        net_exposure: dec_str(net),
        net_exposure_pct_of_equity: net_pct,
        // A one-day VaR needs a return history the platform does not retain yet.
        // Reporting `null` is the honest answer; the panel renders "not computed".
        var95_one_day: None,
        margin_used_pct: margin_pct,
        margin_limit_pct: MARGIN_LIMIT_PCT,
        concentration_top_pct: concentration,
        within_limits: breaches.is_empty(),
        breaches,
        kill_switch_tripped: tripped,
    })
    .into_response()
}

// ── price alerts ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAlertRequest {
    #[serde(alias = "instrumentId")]
    instrument_id: String,
    kind: String,
    value: String,
    note: Option<String>,
    #[serde(default)]
    channels: Option<Vec<String>>,
}

/// `GET /api/alerts` — every alert this user has set.
pub async fn list_alerts(token: BearerToken, State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Decimal, Option<String>, bool, String, DateTime<Utc>, Option<DateTime<Utc>>, Option<Decimal>)>(
        r#"
        SELECT alert_id, instrument_id, kind, value, note, active, channels,
               created_at, triggered_at, triggered_price
        FROM price_alerts
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(token.user_id())
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => Json(json!({
            "alerts": rows.into_iter().map(|r| json!({
                "id": r.0,
                "instrumentId": r.1,
                "kind": r.2,
                "value": dec_str(r.3),
                "note": r.4,
                "active": r.5,
                "channels": r.6.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>(),
                "createdAt": r.7,
                "triggeredAt": r.8,
                "triggeredPrice": r.9.map(dec_str),
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "alerts_query_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/alerts` — set one.
pub async fn create_alert(
    token: BearerToken,
    State(state): State<AppState>,
    Json(req): Json<CreateAlertRequest>,
) -> impl IntoResponse {
    let value: Decimal = match req.value.parse() {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_value", "message": "value must be a decimal" })),
            )
                .into_response();
        }
    };
    if !matches!(
        req.kind.as_str(),
        "price_above" | "price_below" | "pct_change" | "indicator_cross"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_kind" })),
        )
            .into_response();
    }

    let id = Uuid::new_v4();
    let channels = req
        .channels
        .unwrap_or_else(|| vec!["inapp".to_owned()])
        .join(",");

    let res = sqlx::query(
        r#"
        INSERT INTO price_alerts (alert_id, user_id, instrument_id, kind, value, note, channels)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(id)
    .bind(token.user_id())
    .bind(&req.instrument_id)
    .bind(&req.kind)
    .bind(value)
    .bind(&req.note)
    .bind(&channels)
    .execute(&state.pg)
    .await;

    match res {
        Ok(_) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "alert_insert_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `DELETE /api/alerts/{id}`
pub async fn delete_alert(
    token: BearerToken,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let res = sqlx::query("DELETE FROM price_alerts WHERE alert_id = $1 AND user_id = $2")
        .bind(id)
        .bind(token.user_id())
        .execute(&state.pg)
        .await;

    match res {
        Ok(r) if r.rows_affected() == 0 => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "alert_delete_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── user settings ────────────────────────────────────────────────────────────

/// `GET /api/settings` — the preference payload that follows this user.
pub async fn get_settings(token: BearerToken, State(state): State<AppState>) -> impl IntoResponse {
    let row = sqlx::query_as::<_, (serde_json::Value,)>(
        "SELECT payload FROM user_settings WHERE user_id = $1",
    )
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await;

    match row {
        Ok(Some((payload,))) => Json(payload).into_response(),
        Ok(None) => Json(json!({})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "settings_query_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `PUT /api/settings` — replace it. The interface owns the shape; the server
/// stores it verbatim so a new preference never needs a migration.
pub async fn put_settings(
    token: BearerToken,
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let res = sqlx::query(
        r#"
        INSERT INTO user_settings (user_id, payload, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (user_id) DO UPDATE
          SET payload = EXCLUDED.payload, updated_at = now()
        "#,
    )
    .bind(token.user_id())
    .bind(&payload)
    .execute(&state.pg)
    .await;

    match res {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "settings_write_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── equity sampler ───────────────────────────────────────────────────────────

/// Records one equity snapshot per asset class.
///
/// Called on an interval by the platform binary. Without this nothing was ever
/// written to `equity_snapshots`, which is why the dashboard curve had no
/// series to draw: the engine knows equity *now* and nothing remembered it.
pub async fn record_equity_snapshot(
    pg: &sqlx::PgPool,
    engine: &execution::paper::PaperTradingEngine,
    account_mode: &str,
) -> Result<(), sqlx::Error> {
    let at = Utc::now();
    for snap in engine.snapshots() {
        let unrealized: Decimal = snap.positions.iter().map(|p| p.unrealized_pnl).sum();
        sqlx::query(
            r#"
            INSERT INTO equity_snapshots
              (at, account_mode, asset_class, currency, equity, cash, used_margin,
               realized_pnl, unrealized_pnl, fees_paid, open_positions)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            "#,
        )
        .bind(at)
        .bind(account_mode)
        .bind(snap.asset_class.as_str())
        .bind(snap.currency)
        .bind(snap.equity)
        .bind(snap.cash)
        .bind(snap.used_margin)
        .bind(snap.realized_pnl)
        .bind(unrealized)
        .bind(snap.fees_paid)
        .bind(snap.positions.iter().filter(|p| !p.quantity.is_zero()).count() as i32)
        .execute(pg)
        .await?;
    }
    Ok(())
}

// ── alert evaluator ──────────────────────────────────────────────────────────

/// Trips any armed alert whose condition the latest mark satisfies.
///
/// Runs on the same interval as the sampler. An alert that only fires while a
/// browser tab happens to be open is not an alert, which is why this lives on
/// the server rather than in the page that created it.
pub async fn evaluate_alerts(
    pg: &sqlx::PgPool,
    engine: &execution::paper::PaperTradingEngine,
) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Decimal)>(
        "SELECT alert_id, instrument_id, kind, value FROM price_alerts WHERE active = TRUE",
    )
    .fetch_all(pg)
    .await?;

    let mut fired = 0_u64;
    for (id, instrument, kind, value) in rows {
        let Some(mark) = engine.mark(&instrument) else {
            continue;
        };
        let px = mark.inner();
        let hit = match kind.as_str() {
            "price_above" => px >= value,
            "price_below" => px <= value,
            // A percentage alert needs a reference the engine does not retain
            // per alert yet, so it is left armed rather than fired wrongly.
            _ => false,
        };
        if !hit {
            continue;
        }
        sqlx::query(
            "UPDATE price_alerts SET active = FALSE, triggered_at = now(), triggered_price = $2 WHERE alert_id = $1",
        )
        .bind(id)
        .bind(px)
        .execute(pg)
        .await?;
        fired += 1;
    }
    Ok(fired)
}

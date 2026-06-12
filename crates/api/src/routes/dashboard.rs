//! Dashboard rollup endpoint.
//!
//! `GET /api/dashboard/rollup?mode=PAPER|LIVE`
//!
//! Returns three-tier P&L and win-rate data. Computed on-demand, never cached.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::rollup::{compute_rollup, MarkPrice, RollupResponse};
use crate::state::AppState;
use storage::ledger::AccountMode;

#[derive(Debug, Deserialize)]
pub struct RollupQuery {
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "PAPER".to_owned()
}

/// `GET /api/dashboard/rollup?mode=PAPER|LIVE`
pub async fn get_rollup(
    State(_state): State<AppState>,
    Query(params): Query<RollupQuery>,
) -> impl IntoResponse {
    let account_mode = match params.mode.to_uppercase().as_str() {
        "PAPER" => AccountMode::Paper,
        "LIVE" => AccountMode::Live,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown mode: {other}") })),
            )
                .into_response();
        }
    };

    // TODO: In a real deployment, fetch lots/closes from Postgres and marks from Redis.
    // For now, return an empty rollup (the FifoEngine and compute logic are tested separately).
    let rollup = RollupResponse {
        mode: account_mode.as_str(),
        realized_pnl_usd: rust_decimal::Decimal::ZERO,
        unrealized_pnl_usd: rust_decimal::Decimal::ZERO,
        win_rate: 0.0,
        by_asset_class: vec![],
    };

    Json(rollup).into_response()
}

/// Build a rollup response from raw slices (used in tests and the task queue).
pub fn rollup_from_slices(
    user_id: uuid::Uuid,
    mode: AccountMode,
    lots: &[storage::pnl::PnlLot],
    closes: &[storage::pnl::PnlClose],
    marks: &[MarkPrice],
    venue_map: &HashMap<String, (String, String)>,
) -> RollupResponse {
    compute_rollup(user_id, mode, lots, closes, marks, venue_map)
}

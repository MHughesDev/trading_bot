pub mod assets;
pub mod backtests;
pub mod orders;
pub mod strategies;
pub mod streams;
pub mod trading;

use axum::{
    routing::{get, post},
    Router,
};

use crate::state::AppState;

/// Build the full API router tree.
pub fn router(state: AppState) -> Router {
    Router::new()
        // Phase 1 data-plane queries
        .route("/api/assets", get(assets::list_assets))
        .route("/api/instruments/:id", get(assets::get_instrument))
        .route("/api/streams/available", get(streams::list_available))
        // Phase 2 stubs
        .route("/api/orders", post(orders::place_order))
        .route("/api/orders/:id", get(orders::get_order))
        .route("/api/strategies", get(strategies::list_strategies))
        .route("/api/strategies", post(strategies::create_strategy))
        .route("/api/backtests", post(backtests::run_backtest))
        .route("/api/trading/status", get(trading::trading_status))
        .with_state(state)
}

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
use crate::ws::live::ws_live;

/// Build the full API router tree.
pub fn router(state: AppState) -> Router {
    Router::new()
        // Phase 1 data-plane queries
        .route("/api/assets", get(assets::list_assets))
        .route("/api/instruments/:id", get(assets::get_instrument))
        .route("/api/streams/available", get(streams::list_available))
        // Phase 2 order flow
        .route("/api/orders", post(orders::place_order))
        .route("/api/orders/:id", get(orders::get_order))
        // Phase 2 kill switch
        .route("/api/trading/status", get(trading::trading_status))
        .route("/api/trading/kill", post(trading::trip_kill_switch))
        .route("/api/trading/resume", post(trading::reset_kill_switch))
        // Phase 3 UI streaming
        .route("/ws/live", get(ws_live))
        .route(
            "/api/ui/subscriptions",
            post(streams::create_ui_subscriptions),
        )
        // Phase 3+ strategy management
        .route("/api/strategies", get(strategies::list_strategies))
        .route("/api/strategies", post(strategies::create_strategy))
        // Phase 4 backtests
        .route("/api/backtests", post(backtests::run_backtest))
        .route("/api/backtests/:id", get(backtests::get_backtest))
        .with_state(state)
}

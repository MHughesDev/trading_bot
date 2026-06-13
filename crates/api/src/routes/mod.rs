pub mod assets;
pub mod automations;
pub mod backtests;
pub mod dashboard;
pub mod orders;
pub mod strategies;
pub mod streams;
pub mod trading;
pub mod venue_health;

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
        // Phase 5 strategy management (create/list/get/start/stop)
        .route(
            "/api/strategies",
            get(strategies::list_strategies).post(strategies::create_strategy),
        )
        .route("/api/strategies/:id/config", get(strategies::get_strategy))
        .route(
            "/api/strategies/:id/start",
            post(strategies::start_strategy),
        )
        .route("/api/strategies/:id/stop", post(strategies::stop_strategy))
        // P2-T05 venue health checks
        .route("/api/venues/:venue/health", get(venue_health::venue_health))
        // P4-T06 dashboard rollup
        .route("/api/dashboard/rollup", get(dashboard::get_rollup))
        // Automations — persisted server-side; paper and live coexist
        .route(
            "/api/automations",
            get(automations::list_automations).post(automations::create_automation),
        )
        .route(
            "/api/automations/:id/arm",
            post(automations::arm_automation),
        )
        .route(
            "/api/automations/:id/disarm",
            post(automations::disarm_automation),
        )
        .route(
            "/api/automations/:id",
            axum::routing::delete(automations::delete_automation),
        )
        // P3-T03 apply-list
        .route("/api/strategies/apply-list", get(strategies::apply_list))
        // Back Testing — simulation runs against the market_simulator engine
        .route(
            "/api/backtests",
            get(backtests::list_backtests).post(backtests::create_backtest),
        )
        .route(
            "/api/backtests/:id",
            get(backtests::get_backtest).delete(backtests::delete_backtest),
        )
        .route("/api/backtests/:id/stop", post(backtests::stop_backtest))
        .route("/api/backtests/:id/rerun", post(backtests::rerun_backtest))
        .with_state(state)
}

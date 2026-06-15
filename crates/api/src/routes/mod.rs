pub mod asset_lifecycle;
pub mod assets;
pub mod automations;
pub mod backtests;
pub mod dashboard;
pub mod orders;
pub mod strategies;
pub mod streams;
pub mod trading;
pub mod models;
pub mod venue_health;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::auth::handlers;
use crate::state::AppState;
use crate::ws::live::ws_live;

use asset_lifecycle as al;

/// Build the full API router tree.
pub fn router(state: AppState) -> Router {
    Router::new()
        // ── Auth ─────────────────────────────────────────────────────────────
        .route("/auth/register", post(handlers::register))
        .route("/auth/login", post(handlers::login))
        .route("/auth/me", get(handlers::me))
        .route("/auth/logout", post(handlers::logout))
        .route("/auth/forgot-password", post(handlers::forgot_password))
        .route("/auth/verify-reset-code", post(handlers::verify_reset_code))
        .route("/auth/reset-password", post(handlers::reset_password))
        // ── Asset lifecycle ───────────────────────────────────────────────────
        .route("/assets/initialized", get(al::list_initialized))
        .route("/assets/lifecycle/{symbol}", get(al::get_lifecycle))
        .route("/assets/init/{symbol}", post(al::init_asset))
        .route("/assets/init/jobs/{job_id}", get(al::get_init_job))
        .route("/assets/lifecycle/{symbol}/start", post(al::start_asset))
        .route("/assets/lifecycle/{symbol}/stop", post(al::stop_asset))
        .route(
            "/assets/strategy/{symbol}",
            get(al::get_asset_strategy)
                .put(al::set_asset_strategy)
                .delete(al::delete_asset_strategy),
        )
        .route(
            "/assets/execution-mode/{symbol}",
            get(al::get_exec_mode)
                .put(al::set_exec_mode)
                .delete(al::delete_exec_mode),
        )
        .route("/assets/models/{symbol}", get(al::get_models))
        .route("/assets/chart/bars", get(al::get_chart_bars))
        .route("/assets/chart/trade-markers", get(al::get_trade_markers))
        // Phase 1 data-plane queries
        .route("/api/assets", get(assets::list_assets))
        .route("/api/instruments/{id}", get(assets::get_instrument))
        .route("/api/streams/available", get(streams::list_available))
        // Phase 2 order flow
        .route("/api/orders", post(orders::place_order))
        .route("/api/orders/{id}", get(orders::get_order))
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
        .route("/api/strategies/{id}/config", get(strategies::get_strategy))
        .route(
            "/api/strategies/{id}/start",
            post(strategies::start_strategy),
        )
        .route("/api/strategies/{id}/stop", post(strategies::stop_strategy))
        // P2-T05 venue health checks
        .route(
            "/api/venues/{venue}/health",
            get(venue_health::venue_health),
        )
        // P4-T06 dashboard rollup
        .route("/api/dashboard/rollup", get(dashboard::get_rollup))
        // Automations — persisted server-side; paper and live coexist
        .route(
            "/api/automations",
            get(automations::list_automations).post(automations::create_automation),
        )
        .route(
            "/api/automations/{id}/arm",
            post(automations::arm_automation),
        )
        .route(
            "/api/automations/{id}/disarm",
            post(automations::disarm_automation),
        )
        .route(
            "/api/automations/{id}",
            delete(automations::delete_automation),
        )
        // P3-T03 apply-list
        .route("/api/strategies/apply-list", get(strategies::apply_list))
        // Back Testing — simulation runs against the market_simulator engine
        .route(
            "/api/backtests",
            get(backtests::list_backtests).post(backtests::create_backtest),
        )
        .route(
            "/api/backtests/{id}",
            get(backtests::get_backtest).delete(backtests::delete_backtest),
        )
        .route("/api/backtests/{id}/stop", post(backtests::stop_backtest))
        .route("/api/backtests/{id}/rerun", post(backtests::rerun_backtest))
        // AI Model Studio -- registry, training, evaluation, promotion, deployment
        .route("/api/models", get(models::list_models).post(models::create_model))
        .route("/api/models/{id}", get(models::get_model).patch(models::patch_model).delete(models::delete_model))
        .route("/api/models/{id}/archive", post(models::archive_model))
        .route("/api/models/{id}/train", post(models::start_train))
        .route("/api/models/{id}/runs", get(models::list_runs))
        .route("/api/models/{id}/runs/{run_id}", get(models::get_run))
        .route("/api/models/{id}/runs/{run_id}/cancel", post(models::cancel_run))
        .route("/api/models/{id}/versions", get(models::list_versions).post(models::register_version))
        .route("/api/models/{id}/versions/{v}/evaluate", post(models::start_eval))
        .route("/api/models/{id}/versions/{v}/promote", post(models::promote))
        .route("/api/models/{id}/versions/{v}/test", post(models::test_inference))
        .route("/api/models/{id}/evaluations", get(models::list_evals))
        .route("/api/models/{id}/aliases", get(models::get_aliases))
        .route("/api/models/{id}/aliases/{alias}/rollback", post(models::rollback))
        .route("/api/models/{id}/deployments", get(models::list_deployments).post(models::create_deployment))
        .route("/api/models/{id}/test-cases", get(models::list_test_cases).post(models::add_test_case))
        .route("/api/models/{id}/lineage", get(models::get_lineage))
        .route("/api/models/{id}/traces", get(models::get_traces))
        .route("/api/models/{id}/used-by", get(models::get_used_by))
        .with_state(state)
}

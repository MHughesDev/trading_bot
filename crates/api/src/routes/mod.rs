pub mod agent_sessions;
pub mod asset_lifecycle;
pub mod assets;
pub mod automations;
pub mod backtests;
pub mod dashboard;
pub mod data;
pub mod experiments;
pub mod jobs;
pub mod llm;
pub mod models;
pub mod models_phase6;
pub mod orders;
pub mod paper;
pub mod platform_health;
pub mod portfolio;
pub mod research;
pub mod strategies;
pub mod streams;
pub mod synthetic;
pub mod trading;
pub mod venue_health;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::auth::handlers;
use crate::state::AppState;
use crate::ws::backtest::ws_backtest_suite;
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
        .route(
            "/auth/service-tokens",
            get(handlers::list_service_tokens).post(handlers::create_service_token),
        )
        .route(
            "/auth/service-tokens/{token_prefix}",
            delete(handlers::delete_service_token),
        )
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
        // ── LLM proxy (Set L Phase 3, AGENT-001 §9) ──────────────────────────
        .route("/llm/v1/messages", post(crate::llm_proxy::messages))
        // ── Data API v2 and projects (Set L Phase 2, DATA-005) ───────────────
        .route(
            "/api/projects",
            post(data::create_project).get(data::list_projects),
        )
        .route("/api/projects/{project_id}", get(data::get_project))
        .route("/api/data/bars", get(data::get_bars))
        .route("/api/data/catalog", get(data::get_catalog))
        .route("/api/data/live/{instrument}", get(data::get_live))
        .route(
            "/api/data/synthetic",
            post(synthetic::create_synthetic).get(synthetic::list_synthetic),
        )
        .route(
            "/api/data/synthetic/{instrument}/truth",
            get(synthetic::get_truth),
        )
        // ── Agent sessions, steering, approvals (Set L Phase 6, COMP-006) ────
        .route("/api/agent/sessions", get(agent_sessions::list_sessions))
        .route(
            "/api/agent/sessions/{session_id}",
            get(agent_sessions::get_session),
        )
        .route(
            "/api/agent/sessions/{session_id}/events",
            get(agent_sessions::session_events),
        )
        .route(
            "/api/agent/sessions/{session_id}/steer",
            post(agent_sessions::steer_session),
        )
        .route(
            "/api/agent/sessions/{session_id}/inbox",
            get(agent_sessions::drain_inbox),
        )
        .route(
            "/api/agent/projects/{project_id}/workspace/files",
            get(agent_sessions::workspace_file),
        )
        .route("/api/agent/usage", get(agent_sessions::get_usage))
        .route("/api/approvals", get(agent_sessions::list_approvals))
        .route(
            "/api/approvals/{approval_id}/answer",
            post(agent_sessions::answer_approval),
        )
        // ── Jobs and artifacts (Set L Phase 1, COMP-005) ─────────────────────
        .route("/api/jobs", post(jobs::submit_job).get(jobs::list_jobs))
        .route("/api/jobs/events", get(jobs::job_events))
        .route("/api/jobs/{job_id}", get(jobs::get_job))
        .route("/api/jobs/{job_id}/cancel", post(jobs::cancel_job))
        .route("/api/artifacts/{handle}", get(jobs::get_artifact))
        .route(
            "/api/artifacts/{handle}/content",
            get(jobs::get_artifact_content),
        )
        // Phase 1 data-plane queries
        .route("/api/assets", get(assets::list_assets))
        .route(
            "/api/market/instruments",
            get(assets::list_market_instruments),
        )
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
        // LLM provider credentials + model listing (internal agent)
        .route("/api/llm/credentials", get(llm::credential_status))
        .route(
            "/api/llm/credentials/{provider}",
            axum::routing::put(llm::save_credential).delete(llm::delete_credential),
        )
        .route("/api/llm/{provider}/models", post(llm::list_models))
        // Internal agent runs (LLM-driven strategy design + backtest loop)
        // Conversations: the agent is a chat now. `/runs` stays readable so a
        // historical run is still inspectable, but nothing starts one from a form.
        .route(
            "/api/agent/conversations",
            get(crate::agent::routes::list_conversations)
                .post(crate::agent::routes::create_conversation),
        )
        // What may be asked to run here, so the local tier is reachable from the
        // product rather than only from an env var and a restart.
        .route(
            "/api/agent/profiles",
            get(crate::agent::routes::list_profiles),
        )
        .route(
            "/api/agent/conversations/{id}",
            get(crate::agent::routes::get_conversation)
                .delete(crate::agent::routes::archive_conversation),
        )
        .route(
            "/api/agent/conversations/{id}/messages",
            post(crate::agent::routes::send_message),
        )
        .route(
            "/api/agent/conversations/{id}/cancel",
            post(crate::agent::routes::cancel_conversation),
        )
        .route("/api/agent/trajectory", post(crate::agent::routes::record_trajectory_step))
        .route("/api/agent/runs", get(crate::agent::routes::list_runs))
        .route("/api/agent/runs/{id}", get(crate::agent::routes::get_run))
        .route(
            "/api/agent/runs/{id}/messages",
            get(crate::agent::routes::get_messages),
        )
        .route(
            "/api/agent/runs/{id}/cancel",
            post(crate::agent::routes::cancel_run),
        )
        // Research (FEAT-003): sweeps over the Backtest Suite, diagnostics,
        // and the selection-rule carry-forward read.
        .route(
            "/api/research/sweeps",
            get(research::list_sweeps).post(research::start_sweep),
        )
        .route("/api/research/sweeps/{id}", get(research::get_sweep))
        .route(
            "/api/research/sweeps/{id}/cancel",
            post(research::cancel_sweep),
        )
        .route(
            "/api/research/diagnostics/{run_id}",
            get(research::get_diagnostics),
        )
        .route(
            "/api/backtest/experiments/{id}/studies/{study_id}/carried-forward",
            get(research::get_carried_forward),
        )
        // P4-T06 dashboard rollup
        .route("/api/dashboard/rollup", get(dashboard::get_rollup))
        // Portfolio, execution, risk, alerts and settings (0042).
        .route(
            "/api/portfolio/equity-curve",
            get(portfolio::equity_curve),
        )
        .route("/api/portfolio/positions", get(portfolio::positions))
        .route("/api/execution/orders", get(portfolio::list_orders))
        .route("/api/execution/fills", get(portfolio::list_fills))
        .route("/api/orders/{id}/cancel", post(portfolio::cancel_order))
        .route("/api/risk/summary", get(portfolio::risk_summary))
        .route(
            "/api/account/transactions",
            get(portfolio::transactions),
        )
        .route(
            "/api/alerts",
            get(portfolio::list_alerts).post(portfolio::create_alert),
        )
        .route("/api/alerts/{id}", delete(portfolio::delete_alert))
        .route(
            "/api/settings",
            get(portfolio::get_settings).put(portfolio::put_settings),
        )
        // Paper-trading data + reset (internal engine; paper mode only)
        .route(
            "/api/paper/instrument/{instrument_id}",
            get(paper::get_instrument_activity),
        )
        .route("/api/paper/reset", post(paper::reset_all))
        .route(
            "/api/paper/accounts/{asset_class}/reset",
            post(paper::reset_account),
        )
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
        .route("/api/backtests/coverage", get(backtests::coverage))
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
        // Backtest Suite (Set J) — honest-evaluation core: experiments, studies,
        // nulls, the staged-gate funnel, the one-shot vault, reconciliation.
        .route("/ws/backtest-suite", get(ws_backtest_suite))
        .route(
            "/api/backtest/calibration",
            get(experiments::suite_calibration),
        )
        .route(
            "/api/backtest/experiments",
            get(experiments::list_experiments).post(experiments::create_experiment),
        )
        .route(
            "/api/backtest/experiments/{id}",
            get(experiments::get_experiment),
        )
        .route(
            "/api/backtest/experiments/{id}/promote",
            post(experiments::promote_experiment),
        )
        .route(
            "/api/backtest/experiments/{id}/retire",
            post(experiments::retire_experiment),
        )
        .route(
            "/api/backtest/experiments/{id}/studies",
            get(experiments::list_studies).post(experiments::run_study),
        )
        .route(
            "/api/backtest/experiments/{id}/nulls",
            get(experiments::null_picker).post(experiments::choose_null),
        )
        .route(
            "/api/backtest/experiments/{id}/funnel",
            get(experiments::get_funnel),
        )
        .route(
            "/api/backtest/experiments/{id}/funnel/advance",
            post(experiments::advance_funnel),
        )
        .route(
            "/api/backtest/experiments/{id}/vault",
            get(experiments::get_vault).post(experiments::run_vault),
        )
        .route(
            "/api/backtest/experiments/{id}/reconcile",
            post(experiments::reconcile),
        )
        // AI Model Studio -- registry, training, evaluation, promotion, deployment
        // NB: static /api/models/* paths must be registered before /api/models/{id}
        // so the literal paths win over the dynamic capture.
        .route("/api/models/for-node", get(models::for_node))
        // ── Platform self-monitoring (§16.2) ─────────────────────────────────
        .route("/api/platform/health", get(platform_health::health))
        .route("/api/platform/gates/{subject}", get(platform_health::gate_stack))
        .route("/api/platform/timeline", get(platform_health::timeline))
        // I-2.11 leaderboard (static path must precede dynamic /{id})
        .route("/api/models/leaderboard", get(models::leaderboard))
        // I-3.1 feature library
        .route("/api/models/feature-sets", get(models::list_feature_sets))
        // I-3.5 feature preview
        .route(
            "/api/models/features/preview",
            post(models::feature_preview),
        )
        // Set I — data quality preview (I-0.7) and walk-forward windows (I-0.11)
        .route("/api/models/data/quality", get(models::data_quality))
        .route("/api/models/data/windows", post(models::data_windows))
        .route(
            "/api/models",
            get(models::list_models).post(models::create_model),
        )
        .route(
            "/api/models/{id}",
            get(models::get_model)
                .patch(models::patch_model)
                .delete(models::delete_model),
        )
        .route("/api/models/{id}/archive", post(models::archive_model))
        .route("/api/models/{id}/train", post(models::start_train))
        .route("/api/models/{id}/runs", get(models::list_runs))
        .route("/api/models/{id}/runs/{run_id}", get(models::get_run))
        .route(
            "/api/models/{id}/runs/{run_id}/cancel",
            post(models::cancel_run),
        )
        .route(
            "/api/models/{id}/versions",
            get(models::list_versions).post(models::register_version),
        )
        .route(
            "/api/models/{id}/versions/{v}/evaluate",
            post(models::start_eval),
        )
        .route(
            "/api/models/{id}/versions/{v}/promote",
            post(models::promote),
        )
        .route(
            "/api/models/{id}/versions/{v}/test",
            post(models::test_inference),
        )
        .route("/api/models/{id}/evaluations", get(models::list_evals))
        .route(
            "/api/models/{id}/evaluations/compare",
            get(models::compare_evals),
        )
        .route(
            "/api/models/{id}/evaluations/{eval_id}",
            get(models::get_eval),
        )
        .route("/api/models/{id}/aliases", get(models::get_aliases))
        .route(
            "/api/models/{id}/aliases/{alias}/rollback",
            post(models::rollback),
        )
        .route(
            "/api/models/{id}/deployments",
            get(models::list_deployments).post(models::create_deployment),
        )
        .route(
            "/api/models/{id}/test-cases",
            get(models::list_test_cases).post(models::add_test_case),
        )
        .route(
            "/api/models/{id}/test-cases/{case_id}",
            axum::routing::delete(models::delete_test_case),
        )
        .route(
            "/api/models/{id}/feature-vector",
            get(models::feature_vector),
        )
        .route("/api/models/{id}/lineage", get(models::get_lineage))
        .route("/api/models/{id}/traces", get(models::get_traces))
        .route("/api/models/{id}/used-by", get(models::get_used_by))
        // I-2.12 evaluation reports
        .route(
            "/api/models/{id}/versions/{v}/report",
            get(models::get_report),
        )
        .route(
            "/api/models/{id}/versions/{v}/report/export",
            get(models::export_report),
        )
        // I-3.9 reproduce-from-hash
        .route(
            "/api/models/{id}/runs/reproduce",
            post(models::reproduce_run),
        )
        // I-3.10 run compare
        .route("/api/models/{id}/runs/compare", get(models::compare_runs))
        // I-5.9/I-5.10 — per-model rolling quality + alerts
        .route("/api/models/{id}/quality", get(models::get_model_quality))
        // I-6.1 — distributional publish contract
        .route("/api/models/{id}/predict", get(models_phase6::predict))
        // I-6.4 — tags (static before dynamic)
        .route(
            "/api/registry/tags/search",
            get(models_phase6::search_by_tag),
        )
        .route(
            "/api/registry/tags/{id}/{kind}",
            get(models_phase6::list_tags).post(models_phase6::add_tag),
        )
        .route(
            "/api/registry/tags/{id}/{kind}/{tag}",
            axum::routing::delete(models_phase6::remove_tag),
        )
        // I-6.4 — annotations
        .route(
            "/api/registry/annots/{id}/{kind}",
            get(models_phase6::get_annotations),
        )
        .route(
            "/api/registry/annots/{id}/{kind}/{key}",
            axum::routing::put(models_phase6::set_annotation),
        )
        // I-6.4 — templates
        .route(
            "/api/registry/templates",
            get(models_phase6::list_templates).post(models_phase6::create_template),
        )
        .route(
            "/api/registry/templates/{id}/fork",
            post(models_phase6::fork_template),
        )
        .with_state(state)
}

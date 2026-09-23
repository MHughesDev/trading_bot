pub mod evidence;
pub mod web_and_skills;
pub mod falsification;
pub mod ledger;
pub mod lifecycle;
pub mod agent_sessions;
pub mod agent_audit;
pub mod agent_cost;
pub mod agent_preview;
pub mod desk_catalogue;
pub mod desk_digest;
pub mod desk_running;
pub mod memory;
pub mod agent_templates;
pub mod asset_lifecycle;
pub mod assets;
pub mod automations;
pub mod backtests;
pub mod dashboard;
pub mod data;
pub mod code;
pub mod dossiers;
pub mod desk_charter;
pub mod research_sessions;
pub mod experiments;
pub mod jobs;
pub mod llm;
pub mod models;
pub mod models_phase6;
pub mod monitor;
pub mod orders;
pub mod paper;
pub mod platform_health;
pub mod portfolio;
pub mod proposals;
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
        // Falsification: the agent's primary output surface. Filing a report is
        // a write that nothing can edit afterwards; verifying one is a separate
        // action the agent's token does not carry.
        .route("/api/falsification", get(falsification::list_reports))
        .route("/api/falsification/kills", post(falsification::report_kill))
        // The research lifecycle (AGENT-013 §7.2, §6.2). Sealing a commitment is
        // not a look and spends no trial; the trial-spending tools refuse until
        // one exists.
        .route("/api/lifecycle/preregistrations", post(lifecycle::seal))
        .route("/api/lifecycle/hypotheses", post(lifecycle::state_hypothesis))
        .route("/api/lifecycle/metrics", get(lifecycle::metrics))
        // The campaign door (AGENT-013 §8.5; S-10.11). A run's registration
        // becomes a campaign; the campaign's HYPOTHESIZE becomes a worker
        // sub-run over a projection of that run's context store.
        // Something a run could not answer, written down deliberately
        // (AGENT-013 §9; S-10.12). The loop writes these at run end too,
        // with the stakes unstated; this is the door that states them.
        // The trial gate's door (ADR-S10-10). It has been calling this since
        // it was written; until 2026-09-17 nothing answered, so every
        // trial-spending tool was refused and the research loop did not run.
        // S-8.8: the one route that runs a program a model wrote. Refuses
        // unless a sandbox is configured; it never falls back to this process.
        .route("/api/code/run", post(code::run))
        // S-8.7: the desk's dossier reader has called these since S-6.17.
        // Continuous research (AGENT-014 §10; S-11.1, S-11.14). Every mutating
        // route here is human-only: an agent token holds read access and
        // nothing else, because a session that could start, approve or unpause
        // itself would have no human in it.
        .route(
            "/api/research/sessions",
            get(research_sessions::list).post(research_sessions::create),
        )
        // Static before dynamic: `current` is the session the calling run is
        // in, so a child does not have to be told an id it could get wrong.
        .route("/api/research/sessions/current", get(research_sessions::current))
        .route("/api/research/sessions/{id}", get(research_sessions::get))
        .route("/api/research/sessions/{id}/pause", post(research_sessions::pause))
        .route("/api/research/sessions/{id}/resume", post(research_sessions::resume))
        .route("/api/research/sessions/{id}/stop", post(research_sessions::stop))
        .route(
            "/api/research/sessions/{id}/agenda",
            get(research_sessions::agenda).post(research_sessions::add_item),
        )
        .route(
            "/api/research/sessions/{id}/agenda/order",
            post(research_sessions::reorder),
        )
        .route(
            "/api/research/sessions/{id}/agenda/{item}/approve",
            post(research_sessions::approve_item),
        )
        .route(
            "/api/research/sessions/{id}/agenda/{item}/veto",
            post(research_sessions::veto_item),
        )
        .route(
            "/api/research/sessions/{id}/agenda/{item}/prioritise",
            post(research_sessions::prioritise_item),
        )
        .route(
            "/api/research/sessions/{id}/charter",
            axum::routing::put(research_sessions::amend_charter),
        )
        .route("/api/research/sessions/{id}/digests", get(research_sessions::digests))
        .route("/api/research/sessions/{id}/events", get(research_sessions::events))
        .route("/api/dossiers/{ticket}", get(dossiers::get))
        .route("/api/dossiers/{ticket}/verify/{claim}", post(dossiers::verify))
        .route(
            "/api/dossiers/{ticket}/comments",
            get(dossiers::comments).post(dossiers::add_comment),
        )
        .route("/api/ledger/reservations", post(ledger::reserve))
        // S-8.4: written by the workflow engine, read by the gates.
        .route(
            "/api/ledger/workflow-verdicts",
            post(ledger::record_verdict),
        )
        .route("/api/lifecycle/questions", post(lifecycle::open_question))
        // What the platform's runs cost in trials against what they
        // produced (AGENT-013 §10; S-10.13). Per tier and workflow,
        // never as one number.
        .route("/api/lifecycle/barren", get(lifecycle::barren_rates))
        .route("/api/lifecycle/campaigns", post(lifecycle::define_campaign))
        .route("/api/lifecycle/campaigns/{campaign_id}", get(lifecycle::get_campaign))
        // The power check runs here, before any trial (§6.3).
        .route("/api/lifecycle/designs", post(lifecycle::bind_design))
        // The only doors into rings 1 and 2 (§6.8). Creating one changes
        // nothing; the envelope is on the human's decision.
        .route("/api/lifecycle/proposals", post(lifecycle::create_proposal))
        // The human's side (AGENT-013 §7.5; S-10.9). The agent creates an
        // inert row; a person decides here, and the registry write happens
        // at that moment as that person.
        .route("/api/lifecycle/proposals", get(proposals::list))
        .route(
            "/api/lifecycle/proposals/{proposal_id}/decision",
            post(proposals::decide),
        )
        .route("/api/falsification/defects", post(falsification::report_defect))
        // Feasibility: neither of these spends a trial, which is the point of
        // asking them before a search rather than after it disappoints.
        .route("/api/research/tradeable-frontier", post(falsification::tradeable_frontier))
        .route("/api/research/oracle-test", post(falsification::oracle_test))
        // Web access (ADR-P7-01). The only two routes in the platform that
        // reach the internet; everything they return is untrusted.
        .route("/api/web/search", post(web_and_skills::search))
        .route("/api/web/fetch", post(web_and_skills::fetch))
        // The Python skill library (ADR-P7-03).
        .route("/api/skills", get(web_and_skills::list_skills).post(web_and_skills::create_skill))
        .route("/api/skills/run", post(web_and_skills::run_skill))
        // The templated tier (S-3.1). One route for every template: which one is
        // a field, not a path, so adding a tool never touches this file.
        .route("/api/agent/templates/run", post(agent_templates::run_template))
        // The honesty arithmetic (S-3.6, S-3.7). `holdout-look` is the only
        // agent-reachable path to holdout rows in the whole platform.
        .route("/api/agent/audit/deflated-sharpe", post(agent_audit::deflated_sharpe))
        .route("/api/agent/audit/min-track-record", post(agent_audit::min_track_record))
        .route("/api/agent/audit/holdout-budget", post(agent_audit::holdout_budget))
        .route("/api/agent/audit/holdout-look", post(agent_audit::holdout_look))
        .route("/api/agent/audit/probability-of-overfit", post(agent_audit::probability_of_overfit))
        .route("/api/agent/audit/family-wise", post(agent_audit::family_wise))
        .route("/api/agent/audit/metric-integrity", post(agent_audit::metric_integrity))
        .route("/api/agent/cost/estimate", post(agent_cost::estimate))
        .route("/api/agent/strategy/preview", post(agent_preview::preview))
        .route("/api/catalogue", get(desk_catalogue::catalogue))
        .route("/api/catalogue/exposure", get(desk_catalogue::exposure))
        .route("/api/desk/digest", get(desk_digest::digest))
        // §2.2 Running now: per-run tier, stage, tokens and cache. The
        // conversation list answers a different question and cannot carry them.
        .route("/api/desk/running", get(desk_running::running))
        // S-8.2: has this hypothesis already been registered?
        .route(
            "/api/lifecycle/hypotheses/similar",
            post(lifecycle::similar_hypotheses),
        )
        // Every method on the bucket surface hangs off the parameter, and no
        // literal is written under `/api/memory/`. `post` was mounted on
        // `/api/memory/preferences` until 2026-09-22, and because matchit prefers
        // a literal segment the literal took the path and refused the method —
        // `GET /api/memory/preferences` answered 405 while its six siblings
        // answered 200, so `memory::list` was never reached for that bucket.
        // `kind_for` enumerates the legitimate values of `{bucket}`, which makes
        // any literal here a second spelling of one.
        .route("/api/memory/{bucket}", get(memory::list).post(memory::record))
        .route("/api/memory/{bucket}/{id}", delete(memory::forget))
        // Evidence for the falsification tools (ADR-P7-05). None of these
        // returns a verdict; each answers one defect class with a number.
        .route("/api/evidence/trades", post(evidence::trades))
        .route("/api/evidence/data-quality", post(evidence::data_quality))
        .route("/api/evidence/similar-trials", post(evidence::similar_trials))
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
        // ── COMP-007: the live monitor and kill switch ───────────────────────
        .route("/api/monitor/kill-switch", get(monitor::get_kill_switch))
        .route(
            "/api/monitor/kill-switch/{scope}/trip",
            post(monitor::trip),
        )
        .route(
            "/api/monitor/kill-switch/{scope}/reset",
            post(monitor::reset),
        )
        .route("/api/monitor/events", get(monitor::list_events))
        .route(
            "/api/monitor/events/{event_id}/label",
            post(monitor::label_breach),
        )
        .route("/api/monitor/strategies", get(monitor::list_strategies))
        .route(
            "/api/monitor/strategies/{strategy_id}/{action}",
            post(monitor::strategy_action),
        )
        .route(
            "/api/monitor/limits/{strategy_id}",
            axum::routing::put(monitor::declare_limit),
        )
        .route(
            "/api/monitor/limits/{strategy_id}/promotion-gate",
            get(monitor::promotion_gate),
        )
        .route("/api/monitor/drill", post(monitor::record_drill))
        .route("/api/monitor/windows", post(monitor::open_window))
        .route(
            "/api/monitor/windows/{window_id}",
            delete(monitor::close_window),
        )
        .route("/api/monitor/precision", get(monitor::get_precision))
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
        // The audit trail's two halves (SPEC §15, AT-64). Every front door opens
        // a record here before it dispatches, including the external MCP process.
        .route("/api/agent/audit", post(crate::agent::routes::open_audit_record))
        .route("/api/agent/audit/{audit_id}", post(crate::agent::routes::close_audit_record))
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
        // The bounded pause (ADR-D-27). Not a third control beside cancel: the
        // same `RunControl` the driver already polls at its step boundaries,
        // taught to hold the loop there instead of only stopping it. The
        // deadline is enforced by that parked task, and on expiry the run stops
        // rather than resuming.
        .route(
            "/api/agent/runs/{id}/pause",
            post(crate::agent::routes::pause_run),
        )
        .route(
            "/api/agent/runs/{id}/resume",
            post(crate::agent::routes::resume_run),
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

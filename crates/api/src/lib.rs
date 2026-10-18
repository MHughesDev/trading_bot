// The workspace policy (Cargo.toml `[workspace.lints.clippy]`) allows these; this crate
// does not opt in to workspace lints, so say it here rather than per call site.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::result_large_err
)]

pub mod accounts;
pub mod agent;
pub mod auditor;
pub mod auth;
pub mod barren;
pub mod campaign_driver;
pub mod compare_candidates;
pub mod context_corpus;
pub mod credentials;
pub mod data_admission;
pub mod dataset_build_worker;
pub mod deployment_gate;
pub mod deployment_gate_routes;
pub mod dossier;
pub mod eval_store;
pub mod feature_consistency;
pub mod features_compute;
pub mod gate_worker;
pub mod instrument_meta;
pub mod knowledge;
pub mod knowledge_jobs;
pub mod knowledge_regime_jump;
pub mod knowledge_routes;
pub mod layer1_reconciliation;
pub mod layer1_worker;
pub mod leakage_audit;
pub mod leakage_lint;
pub mod legacy_marker;
pub mod lifecycle_evaluate;
pub mod live_bus;
pub mod llm_proxy;
pub mod memory_store;
pub mod model_call;
pub mod model_policy;
pub mod models_usage_policy;
pub mod negative_from_funnel;
pub mod orchestrator;
pub mod projects;
pub mod questions;
pub mod ramp_worker;
pub mod regime_separation;
pub mod report_validator;
pub mod repro_drift;
pub mod research;
pub mod research_run_worker;
pub mod retrieval_store;
pub mod risk_context;
pub mod rollup;
pub mod routes;
pub mod self_monitor;
pub mod session_contradictions;
pub mod session_day;
pub mod session_dedupe;
pub mod session_driver;
pub mod session_profile;
pub mod session_regime;
pub mod session_survey;
pub mod session_wake;
pub mod sessions;
pub mod skills_store;
pub mod skills_verify;
pub mod state;
pub mod synthetic_worker;
pub mod trace_from_effects;
pub mod turn_summary_store;
pub mod workers;
pub mod ws;

pub use routes::router;
pub use state::{AppState, StreamRequest};

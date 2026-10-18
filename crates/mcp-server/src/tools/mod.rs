//! MCP tool handlers — thin wrappers over the platform's HTTP API (ADR-0010).
//!
//! # No order-placement tool
//!
//! There is no `place_order` tool. Strategies authored here emit order intents
//! only when running on the runtime, and those intents pass through the risk gate
//! like any other path.

pub mod alpha_chain;
pub mod assets;
pub mod audit;
pub mod authoring;
pub mod automations;
pub mod backtests;
pub mod builder;
pub mod campaign;
pub mod code;
pub mod control_limits;
pub mod core;
pub mod deployment_proposals;
pub mod discovery;
pub mod explain_trade;
pub mod falsification;
pub mod layer1;
pub mod lifecycle;
pub mod lifecycle_sessions;
pub mod market;
pub mod models;
pub mod path_risk;
pub mod portfolio;
pub mod research;
pub mod research_tools;
pub mod robustness;
pub mod knowledge_model;
pub mod skills_registry;
pub mod selections;
pub mod strategy;
pub mod strategy_v3;
pub mod templated;
pub mod venues;
pub mod workflows;

// Set S phase 5 (S-5.8, S-5.11–S-5.13, S-5.15): the SLv2 tools that need more
// than one pure call, and the F2 cross-language reconciliation.
pub mod strategy_lang;
#[cfg(test)]
mod features_f2_reconciliation;
#[cfg(test)]
mod features_registry_parity;
#[cfg(test)]
mod moves_roundtrip;

//! MCP tool handlers — thin wrappers over the platform's HTTP API (ADR-0010).
//!
//! # No order-placement tool
//!
//! There is no `place_order` tool. Strategies authored here emit order intents
//! only when running on the runtime, and those intents pass through the risk gate
//! like any other path.

pub mod assets;
pub mod authoring;
pub mod automations;
pub mod backtests;
pub mod builder;
pub mod discovery;
pub mod market;
pub mod models;
pub mod portfolio;
pub mod research;

//! MCP tool handlers — thin wrappers over the platform's business logic.
//!
//! # No order-placement tool
//!
//! There is no `place_order` tool. Strategies authored here emit order intents
//! only when running on the runtime, and those intents pass through the risk gate
//! like any other path.

pub mod authoring;
pub mod discovery;
pub mod lifecycle;

//! Internal agent: an LLM-driven loop that designs trading strategies,
//! launches backtests, waits on them server-side, reads results, and iterates.
//!
//! Tool execution goes through the platform's own HTTP API on loopback with a
//! run-scoped service token — the identical code path (`mcp-server-lib`) the
//! external MCP front door uses, so tool behavior cannot drift between the two.

pub mod audit;
pub mod bridge;
pub mod conversations;
pub mod driver;
pub mod hardware_probe;
pub mod local_driver;
pub mod manager;
pub mod prompt;
pub mod routes;
pub mod workspace_tools;

pub use manager::{AgentManager, StartError};

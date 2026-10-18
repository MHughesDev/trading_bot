//! Internal agent: an LLM-driven loop that designs trading strategies,
//! launches backtests, waits on them server-side, reads results, and iterates.
//!
//! Tool execution goes through the platform's own HTTP API on loopback with a
//! run-scoped service token — the identical code path (`mcp-server-lib`) the
//! external MCP front door uses, so tool behavior cannot drift between the two.

pub mod audit;
pub mod bridge;
pub mod control;
pub mod conversations;
pub mod hardware_probe;
pub mod local_driver;
pub mod manager;
pub mod prompt;
pub mod resume;
pub mod routes;
pub mod skills_mount;
pub mod subrun;
pub mod workspace_tools;

/// The one driver, under the name the spec gives it (AGENT-010 §2; S-2.10).
///
/// `driver.rs` — the hosted provider loop — is deleted; every tier runs on
/// `local_driver`. This alias keeps `agent::driver::truncate_result` resolving
/// for `crate::auditor`, which is outside this module's territory; once that
/// call site names `local_driver` the alias can go.
pub use local_driver as driver;

pub use control::{Hold, PauseRefused, PauseWindow, ResumeRefused, RunControl, StopReason};
pub use manager::{AgentManager, StartError, TurnOptions};

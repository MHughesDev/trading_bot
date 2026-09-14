//! The agent harness (ADR-0031, `AGENT_HARNESS_GUIDE_v3.md`).
//!
//! One harness, parameterised by a capability profile. The model is a plug-in behind
//! the Model Adapter, and nothing in this crate knows which provider is in use — so
//! the same tool implementations, the same budgets and the same validation run
//! whether the executor is the Claude Agent SDK or a locally hosted open-weight
//! model added later.
//!
//! # Why this lives at the platform, not in the container
//!
//! The container is the thing being contained (D-10). Enforcement that runs inside
//! it is enforcement the agent can reach. So the guide's harness-side components sit
//! here, above the executor:
//!
//! | Guide component | Module |
//! |---|---|
//! | Tool Registry | [`registry`] |
//! | Context Manager | [`context`] |
//! | Model Adapter | [`adapter`] |
//! | Execution Loop / State Store | the job service and orchestrator (`crates/jobs`, `crates/api`) |
//! | Observability | the trace in [`trace`] plus `agent_events` |
//!
//! Plus two the guide requires that had no home before: [`policy`] (risk ×
//! archetype × provenance → allow/ask/deny) and [`provenance`] (tagging and
//! untrusted-content wrapping).
//!
//! # The one rule that keeps this honest
//!
//! **A profile value that nothing enforces is a bug.** It reads as a guarantee and
//! behaves as a comment. `tests/conformance.rs` asserts every field is read by some
//! code path, and it fails when a field is added without an enforcement point.

pub mod adapter;
pub mod appendix_a;
pub mod canary;
pub mod context;
pub mod drive;
pub mod hardware;
pub mod policy;
pub mod profile;
pub mod provenance;
pub mod registry;
pub mod trace;
pub mod validation;
pub mod workspace;

pub use drive::{Degradation, Effect, Input, Loop, Outcome, State, Task};
pub use policy::{decide, ActionContext, Decision, Ruling};
pub use profile::{Archetype, Profile, ProfileSet, Tier};
pub use provenance::{Provenance, Tagged};
pub use registry::{Risk, ToolDef, ToolRegistry};
pub use workspace::{Area, Delivery, Workspace};

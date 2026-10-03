//! The one strategy engine (FEAT-007, ADR-0039).
//!
//! Pure, synchronous and deterministic. Every environment — backtest, preview,
//! paper, live — drives the same code: it compiles a `tbot.strategy/3` graph
//! ([`compile`]), feeds an [`Engine`] events and a portfolio view, answers its
//! model requests, and receives decisions. Nothing in this crate reads a clock, a
//! socket, a database or a file, and it keeps no guess of its own position.
//!
//! - [`compile`] — the only typechecker for strategies; teachable diagnostics.
//! - [`Engine`] — events in, [`Step`]s out; models by request and resume.
//! - [`SelectionEngine`] — the same nodes over every member of a universe,
//!   compared across members and ranked into a selection.
//! - [`planner`] — trade intents into orders, identically everywhere.
//! - [`trace`] — what every step leaves behind, byte-stable for parity checks.
//! - [`env`] — the ports an environment implements, and [`cycle`], the one loop
//!   that drives the engine through them.

pub mod allocation;
pub mod checkpoint;
pub mod compile;
pub mod decide;
pub mod diagnostic;
pub mod engine;
pub mod env;
pub mod eval;
mod history;
mod instance;
pub mod models;
pub mod plan;
pub mod planner;
pub mod portfolio;
pub mod registry;
pub mod schedule;
pub mod selection;
pub mod temporal;
pub mod trace;

pub use checkpoint::{Checkpoint, RestoreError};
pub use compile::compile;
pub use diagnostic::Diagnostic;
pub use engine::{Engine, EngineError, Event, Step};
pub use env::{
    cycle, Clock, Journal, MarketData, MarketEvent, Membership, ModelProvider, VenuePort,
};
pub use history::Bar;
pub use models::{ModelInput, ModelOutcome, ModelRequest, ModelResponse};
pub use plan::Plan;
pub use portfolio::{
    Fill, InstrumentSpec, OrderClosed, OrderClosedReason, PortfolioView, VenueCaps, WorkingOrder,
};
pub use registry::{Registries, ResolvedModel, StdRegistries};
pub use selection::{SelectionEngine, SelectionRun, SelectionStep};
pub use trace::{Cause, DecisionTrace};

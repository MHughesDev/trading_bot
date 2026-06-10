//! Strategy execution engine — `WorldState`, interpreter, intent emission.
//!
//! Consumes canonical events (never the UI feed).  The same interface runs live
//! and in replay; determinism is enforced by `world.now()` returning
//! `available_time` rather than the OS clock.

pub mod automation;
pub mod clock;
pub mod compatibility;
pub mod intents;
pub mod interpreter;
pub mod kind;
pub mod manifest;
pub mod nodes;
pub mod runtime;
pub mod world;

pub use clock::{ReplayClock, StrategyClock, WallClock};
pub use interpreter::{evaluate_condition, evaluate_signals, EvalError};
pub use runtime::{InstanceManager, RuntimeError, StrategyInstance};
pub use world::{StrategyResult, WorldContext, WorldEvent, WorldState};

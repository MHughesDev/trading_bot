//! Strategy execution engine — `WorldState`, interpreter, intent emission.
//!
//! Consumes canonical events (never the UI feed).  The same interface runs live
//! and in replay; determinism is enforced by `world.now()` returning
//! `available_time` rather than the OS clock.

pub mod exits;
pub mod automation;
pub mod bytecode;
pub mod clock;
pub mod compatibility;
pub mod history;
pub mod ids;
pub mod intents;
pub mod interpreter;
pub mod kind;
pub mod live;
pub mod manifest;
pub mod nodes;
pub mod registry;
pub mod runtime;
pub mod world;

pub use bytecode::{compile as compile_expr, run as run_expr, BarField, Op, Program};
pub use clock::{ReplayClock, StrategyClock, WallClock};
pub use history::{
    BarTransition, HistoryRegisters, SignalVerdicts, Unknown, Unresolved, Verdict,
    MAX_HISTORY_DEPTH,
};
pub use intents::{SizedIntents, SizingContext, SizingError};
pub use live::{LiveKey, LiveRegistryError, LiveSource, LiveStrategyRegistry, LiveStrategySpec};
pub use interpreter::{
    evaluate_condition, evaluate_signals, nodes_history_warmup_bars, nodes_need_history, EvalError,
    SignalEvaluator,
};
pub use registry::FeatureRegistry;
// Re-exported so a caller can build a `WorldEvent::Feature` without
// depending on `features` directly.
pub use features::FeatureValue;
pub use runtime::{InstanceManager, RuntimeError, StrategyInstance};
pub use world::{StrategyResult, WorldContext, WorldEvent, WorldState};

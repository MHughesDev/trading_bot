//! TODO(Phase 4): strategy execution engine — WorldState, interpreter, intent emission.
//! Consumes canonical events (never the UI feed). Same interface live and in backtest.
pub mod clock;
pub mod intents;
pub mod interpreter;
pub mod runtime;
pub mod world;

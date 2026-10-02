//! The single risk gate — every order passes through here with no bypass.

pub mod context;
pub mod gate;
pub mod kill_switch;
pub mod limits;
pub mod overrides;
pub mod rate;
pub mod strategy_halt;
pub mod trust_gate;

pub use context::{RiskContextProvider, SimulatedInputs, SimulatedRiskState};
pub use gate::{ApprovedOrder, GateContext, InstrumentLimits, Measured, RateSource, RiskGate};
pub use kill_switch::{KillSwitch, LeaseGate, Scope, UnknownScope};
pub use limits::GlobalRiskLimits;
pub use rate::{RateCounts, RateKey, RateTracker};
pub use strategy_halt::{
    ClearError, DivergenceHalt, DivergenceKind, HaltClearance, HumanActor, StrategyHaltRegistry,
};

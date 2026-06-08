//! The single risk gate — every order passes through here with no bypass.

pub mod gate;
pub mod kill_switch;
pub mod limits;
pub mod overrides;
pub mod trust_gate;

pub use gate::{ApprovedOrder, GateContext, RiskGate};
pub use kill_switch::KillSwitch;
pub use limits::GlobalRiskLimits;

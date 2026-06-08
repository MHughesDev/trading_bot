//! The single risk gate — every order passes through here with no bypass.
//!
//! TODO(Phase 2): implement RiskGate, limits, kill switch, tighten-only overrides.
pub mod gate;
pub mod kill_switch;
pub mod limits;
pub mod overrides;
pub mod trust_gate;

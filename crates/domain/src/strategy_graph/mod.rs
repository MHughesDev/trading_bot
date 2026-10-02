//! Strategies as typed dataflow graphs — `tbot.strategy/3` (FEAT-007, ADR-0040).
//!
//! Data only. The graph ([`graph`]), the values on its wires ([`value`]), what a
//! trading strategy asks for ([`intent`]), what a selection strategy returns
//! ([`selection`]), where a strategy is deployed ([`deployment`]), and the table
//! of operators a graph may use ([`ops`]).
//!
//! The v1 format in [`crate::strategy_def`] remains readable and translates to
//! this one (FEAT-007 §12); nothing new is built on it.

pub mod deployment;
pub mod graph;
pub mod intent;
pub mod ops;
pub mod selection;
pub mod value;

pub use deployment::{
    DeploymentEnvironment, DeploymentSpec, DeploymentTarget, OnRemoval, Weighting,
};
pub use graph::{
    Arg, DataInput, Decide, GraphError, InstrumentRef, NodeDef, RefPath, RiskLimits, Rule, Select,
    Sink, StrategyGraph, StrategyKind, TimeframeKey, Triggers, UniverseRef, FORMAT,
};
pub use intent::{
    Bracket, Distance, DistanceUnit, Entry, OrderSpec, OrderStyle, Protect, SizeSpec, TradeIntent,
};
pub use selection::{ExclusionReason, Selection};
pub use value::{Level, MissingReason, Num, Truth, Value, ValueType};

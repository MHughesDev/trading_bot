//! Action types — maps named signals to order intents.
//!
//! All order intents produced by actions route through the risk gate before
//! execution.  The strategy runtime has no private path to a broker.

use serde::{Deserialize, Serialize};

use crate::order::Side;

/// How the order quantity is determined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeMode {
    /// Literal decimal quantity (e.g. `"0.01"` BTC).
    Fixed,
    /// Fraction of the available account balance (future — v1.0 parse-only).
    PercentOfBalance,
    /// R-multiple risk units (future — v1.0 parse-only).
    RiskUnit,
    /// v1.1: size fraction provided by a `Sizing` node (RiskSizing model).
    ///
    /// `node_ref` is the `id` of a `NodeKind::Sizing` node in the same
    /// definition.  The runtime looks up the model's predicted `size_fraction`
    /// (decimal string from the sidecar), parses it to `Decimal` (ADR-0002),
    /// and uses it as the order quantity.
    Model {
        /// ID of the `Sizing` node that provides the fraction.
        node_ref: String,
    },
}

/// The order specification embedded in a `PlaceOrder` action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSpec {
    pub side: Side,
    pub size_mode: SizeMode,
    /// Decimal string — the literal size for `Fixed`, the fraction for
    /// `PercentOfBalance`, or the R-multiple for `RiskUnit`.
    pub size: String,
}

/// An action triggered by a named signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Name of the signal this action reacts to (e.g. `"long"`).
    pub on_signal: String,
    /// Action type — currently only `place_order` is supported in v1.0.
    #[serde(flatten)]
    pub kind: ActionKind,
}

/// Discriminated action type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    PlaceOrder { order: OrderSpec },
}

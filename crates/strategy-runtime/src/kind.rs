//! Strategy kind inference — `Discovery` vs `Execution`.
//!
//! Kind is never declared; it is computed from the strategy graph.
//! A strategy with a `PlaceOrder` execution block → `Execution`.
//! Everything else → `Discovery` (populates scanner panels).

use domain::strategy_def::{actions::ActionKind, StrategyDefinition};
use serde::{Deserialize, Serialize};

/// The inferred runtime role of a compiled strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// Populates scanner/watchlist panels.  No orders are placed.
    Discovery,
    /// Runs inside Automations; places orders when the final condition fires.
    Execution,
}

/// Infer the strategy kind from the definition graph.
///
/// Rule: if any action contains a `PlaceOrder` execution block → `Execution`;
/// otherwise → `Discovery`.  A stored `strategy_type` field (if present in
/// legacy JSON) is ignored — the inferred value always wins (C-061).
impl StrategyKind {
    /// Lowercase string key — avoids `format!("{:?}", kind).to_lowercase()` allocations.
    pub fn as_str(self) -> &'static str {
        match self {
            StrategyKind::Discovery => "discovery",
            StrategyKind::Execution => "execution",
        }
    }
}

pub fn infer_kind(def: &StrategyDefinition) -> StrategyKind {
    let has_execution_block = def
        .actions
        .iter()
        .any(|a| matches!(a.kind, ActionKind::PlaceOrder { .. }));

    if has_execution_block {
        StrategyKind::Execution
    } else {
        StrategyKind::Discovery
    }
}

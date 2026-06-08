//! `InputDeclaration` — lane subscription declared in a strategy definition.
//!
//! # `$bound_at_init` semantics
//!
//! The `instrument` field in a definition stores the literal string
//! `"$bound_at_init"` as a placeholder.  When the user initializes a strategy
//! on a specific instrument, the runtime resolves the placeholder to the actual
//! instrument ID and stores that in the *instance* record.  The definition
//! retains the placeholder and remains reusable across instruments.

use serde::{Deserialize, Serialize};

/// Sentinel value used in definitions where the instrument is resolved at
/// instance initialization time.
pub const BOUND_AT_INIT: &str = "$bound_at_init";

/// A single lane subscription entry in `StrategyDefinition.inputs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDeclaration {
    /// NATS lane (e.g. `"market.bars.1m"`, `"features.technical"`).
    pub lane: String,
    /// Either the literal `"$bound_at_init"` or a hardcoded instrument ID.
    pub instrument: String,
    /// Specific feature names needed from `features.*` lanes.  Empty for
    /// non-feature lanes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

impl InputDeclaration {
    pub fn is_bound_at_init(&self) -> bool {
        self.instrument == BOUND_AT_INIT
    }
}

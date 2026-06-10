//! Strategy manifest persistence — writes and reads compiled `CapabilityManifest`
//! records from the `strategy_manifests` table (migration 0009).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Row shape for the `strategy_manifests` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyManifestRow {
    pub strategy_id: Uuid,
    /// JSON array of DataType dotted-key strings, e.g. `["market.ohlcv"]`.
    pub required_lanes: serde_json::Value,
    pub required_primitives: serde_json::Value,
    pub required_features: serde_json::Value,
    /// `"bar_close"` | `"tick"` | `"quote"` | `"event"` | `"scheduled"`.
    pub evaluation_trigger: String,
    /// `"discovery"` | `"execution"`.
    pub strategy_kind: String,
    pub compiled_at: DateTime<Utc>,
}

//! Automation plan persistence — `automations` and `automation_stage_membership`
//! tables (migration 0010).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Row shape for the `automations` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRow {
    pub id: Uuid,
    pub user_id: Uuid,
    /// `"single_instrument"` | `"pipeline"`.
    pub kind: String,
    /// `"paper"` | `"live"`.
    pub account_mode: String,
    /// Full `AutomationSpec` serialized as JSONB.
    pub spec: serde_json::Value,
    pub armed: bool,
    pub created_at: DateTime<Utc>,
}

/// Row shape for the `automation_stage_membership` table.
///
/// Primary key: `(automation_id, stage_id, instrument_id)` — enforces that
/// each instrument appears at most once per stage per automation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct StageMembershipRow {
    pub automation_id: Uuid,
    pub stage_id: String,
    pub instrument_id: String,
    pub entered_at: DateTime<Utc>,
}

impl StageMembershipRow {
    /// Construct a new membership row timestamped now.
    pub fn new(
        automation_id: Uuid,
        stage_id: impl Into<String>,
        instrument_id: impl Into<String>,
    ) -> Self {
        Self {
            automation_id,
            stage_id: stage_id.into(),
            instrument_id: instrument_id.into(),
            entered_at: Utc::now(),
        }
    }
}

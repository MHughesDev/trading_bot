//! Automation plan persistence — `automations` and `automation_stage_membership`
//! tables (migration 0010).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::postgres::PgPool;

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

fn row_to_automation(row: &sqlx::postgres::PgRow) -> AutomationRow {
    AutomationRow {
        id: row.get("id"),
        user_id: row.get("user_id"),
        kind: row.get("kind"),
        account_mode: row.get("account_mode"),
        spec: row.get("spec"),
        armed: row.get("armed"),
        created_at: row.get("created_at"),
    }
}

/// Insert a new automation plan row.
pub async fn insert_automation(pool: &PgPool, row: &AutomationRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO automations (id, user_id, kind, account_mode, spec, armed, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(row.id)
    .bind(row.user_id)
    .bind(&row.kind)
    .bind(&row.account_mode)
    .bind(&row.spec)
    .bind(row.armed)
    .bind(row.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// All automation rows, newest first.  Paper and live automations are both
/// returned — they coexist server-side regardless of which mode a UI session
/// is displaying.
pub async fn list_automations(pool: &PgPool) -> Result<Vec<AutomationRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, user_id, kind, account_mode, spec, armed, created_at \
         FROM automations ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_automation).collect())
}

/// All armed automation rows (used to resume automations at server startup).
pub async fn armed_automations(pool: &PgPool) -> Result<Vec<AutomationRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, user_id, kind, account_mode, spec, armed, created_at \
         FROM automations WHERE armed ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_automation).collect())
}

/// Arm or disarm one automation.  Returns `true` when a row was updated.
pub async fn set_automation_armed(
    pool: &PgPool,
    id: Uuid,
    armed: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE automations SET armed = $2 WHERE id = $1")
        .bind(id)
        .bind(armed)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Delete one automation (stage membership cascades).  Returns `true` when a
/// row was deleted.
pub async fn delete_automation(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM automations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Set `armed = false` for every automation.  Called at server startup so no
/// automation runs until the user explicitly re-arms it.  Returns the number
/// of rows updated.
pub async fn disarm_all_automations(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("UPDATE automations SET armed = false WHERE armed = true")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Fetch a single automation row by id.
pub async fn get_automation(pool: &PgPool, id: Uuid) -> Result<Option<AutomationRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, user_id, kind, account_mode, spec, armed, created_at \
         FROM automations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_automation))
}

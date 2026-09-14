//! Agent trajectory logging (SPEC §14.6): every tool step an agent takes, with the
//! exact schema it was offered, so a future fine-tune or critic can be trained on
//! what actually happened. Append-only; critic labels arrive later as separate rows.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::pg::{tenant_tx, PgTrialLedger};
use crate::LedgerError;

/// Largest result payload stored verbatim; beyond this a summary is kept.
pub const RESULT_SUMMARY_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub traj_id: Uuid,
    pub step_idx: i32,
    #[serde(default)]
    pub campaign_id: Option<Uuid>,
    pub tool_name: String,
    pub tool_schema_hash: String,
    pub tool_semver: String,
    #[serde(default)]
    pub arguments: Option<Value>,
    #[serde(default)]
    pub result_summary: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub latency_ms: Option<i32>,
    #[serde(default)]
    pub tokens_in: Option<i32>,
    #[serde(default)]
    pub tokens_out: Option<i32>,
    #[serde(default)]
    pub propensity: Option<f64>,
    #[serde(default)]
    pub exploration_flag: Option<bool>,
    #[serde(default)]
    pub outcome_trial_id: Option<Uuid>,
}

impl TrajectoryStep {
    /// # Errors
    /// A step that could not be used to reconstruct what the agent saw.
    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.step_idx < 0 {
            return Err(LedgerError::Invalid("step_idx must be >= 0".into()));
        }
        if self.tool_name.trim().is_empty() {
            return Err(LedgerError::Invalid("tool_name is required".into()));
        }
        if !self.tool_schema_hash.starts_with("sha256:") || self.tool_schema_hash.len() != 71 {
            return Err(LedgerError::Invalid("tool_schema_hash must be sha256:<64 hex>".into()));
        }
        if self.tool_semver.trim().is_empty() {
            return Err(LedgerError::Invalid("tool_semver is required".into()));
        }
        if let Some(p) = self.propensity {
            if !(p > 0.0 && p <= 1.0) {
                return Err(LedgerError::BadPropensity(p));
            }
        }
        Ok(())
    }
}

/// Bound a tool result for storage: verbatim when small, otherwise its size and a
/// prefix, marked as truncated so nobody mistakes it for the whole payload.
#[must_use]
pub fn summarize_result(result: &Value) -> Value {
    let text = result.to_string();
    if text.len() <= RESULT_SUMMARY_BYTES {
        return result.clone();
    }
    let mut cut = RESULT_SUMMARY_BYTES / 2;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    serde_json::json!({ "truncated": true, "bytes": text.len(), "head": &text[..cut] })
}

impl PgTrialLedger {
    /// Append one step. A replayed `(traj_id, step_idx)` is refused by the key.
    ///
    /// # Errors
    /// Validation and backend failures.
    pub async fn record_trajectory_step_async(&self, tenant_id: &str, step: &TrajectoryStep) -> Result<(), LedgerError> {
        step.validate()?;
        let be = |e: sqlx::Error| LedgerError::Backend(e.to_string());
        let mut tx = tenant_tx(self.pool(), tenant_id).await.map_err(be)?;
        sqlx::query(
            "INSERT INTO mlops.agent_trajectory (traj_id, step_idx, tenant_id, campaign_id, tool_name, tool_schema_hash,
                 tool_semver, arguments, result_summary, error, latency_ms, tokens_in, tokens_out, propensity,
                 exploration_flag, outcome_trial_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(step.traj_id)
        .bind(step.step_idx)
        .bind(tenant_id)
        .bind(step.campaign_id)
        .bind(&step.tool_name)
        .bind(&step.tool_schema_hash)
        .bind(&step.tool_semver)
        .bind(&step.arguments)
        .bind(step.result_summary.as_ref().map(summarize_result))
        .bind(&step.error)
        .bind(step.latency_ms)
        .bind(step.tokens_in)
        .bind(step.tokens_out)
        .bind(step.propensity)
        .bind(step.exploration_flag)
        .bind(step.outcome_trial_id)
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(())
    }

    /// Append a critic label for an existing step.
    ///
    /// # Errors
    /// Unknown labels, unknown steps, and backend failures.
    pub async fn label_trajectory_step_async(
        &self,
        tenant_id: &str,
        traj_id: Uuid,
        step_idx: i32,
        label: &str,
        labeled_by: &str,
    ) -> Result<(), LedgerError> {
        if !matches!(label, "good" | "unnecessary" | "mistake" | "recover") {
            return Err(LedgerError::Invalid(format!("unknown critic label {label}")));
        }
        let be = |e: sqlx::Error| LedgerError::Backend(e.to_string());
        let mut tx = tenant_tx(self.pool(), tenant_id).await.map_err(be)?;
        sqlx::query("INSERT INTO mlops.trajectory_label (traj_id, step_idx, tenant_id, critic_label, labeled_by) VALUES ($1,$2,$3,$4,$5)")
            .bind(traj_id)
            .bind(step_idx)
            .bind(tenant_id)
            .bind(label)
            .bind(labeled_by)
            .execute(&mut *tx)
            .await
            .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step() -> TrajectoryStep {
        TrajectoryStep {
            traj_id: Uuid::nil(),
            step_idx: 0,
            campaign_id: None,
            tool_name: "get_bars".into(),
            tool_schema_hash: format!("sha256:{}", "a".repeat(64)),
            tool_semver: "0.1.0".into(),
            arguments: None,
            result_summary: None,
            error: None,
            latency_ms: Some(3),
            tokens_in: None,
            tokens_out: None,
            propensity: None,
            exploration_flag: None,
            outcome_trial_id: None,
        }
    }

    #[test]
    fn a_step_without_its_schema_hash_is_refused() {
        assert!(step().validate().is_ok());
        assert!(TrajectoryStep { tool_schema_hash: String::new(), ..step() }.validate().is_err());
        assert!(TrajectoryStep { tool_schema_hash: "sha256:short".into(), ..step() }.validate().is_err());
        assert!(TrajectoryStep { step_idx: -1, ..step() }.validate().is_err());
        assert!(TrajectoryStep { propensity: Some(0.0), ..step() }.validate().is_err());
    }

    #[test]
    fn large_results_are_summarized_and_marked() {
        let small = serde_json::json!({ "ok": 1 });
        assert_eq!(summarize_result(&small), small);
        let big = serde_json::json!({ "rows": "é".repeat(RESULT_SUMMARY_BYTES) });
        let s = summarize_result(&big);
        assert_eq!(s["truncated"], true);
        assert!(s["bytes"].as_u64().unwrap() > RESULT_SUMMARY_BYTES as u64);
    }
}

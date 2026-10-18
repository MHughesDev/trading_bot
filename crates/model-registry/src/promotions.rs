//! Alias promotion is a person's decision (Set N N-4.3, MD-13, AGENT-013 ring 1).
//!
//! `promote_gated` moves the `production` alias. That is a ring-1 write, so an agent can
//! only *propose* it: [`propose`] writes an inert `pending` row carrying the evidence a
//! decider needs, and [`decide`] — reachable only from a human session, enforced at the
//! route — records the decision and, on approval, performs the promotion *as that person*.
//!
//! Three facts are kept apart on the row, as for every proposal in the platform:
//! **decided** (a person said yes or no), **performed** (the platform then moved the alias)
//! and **effect** (what it did). An approval whose promotion the gate refuses is
//! `approved` and not performed, and says so; the schema's CHECK refuses `performed_at` on
//! anything that was not approved.
//!
//! ADR-N-08: this is a model-promotion table of its own, not a new `ProposalKind` in
//! `crates/ledger`. The ledger's kinds carry strategy and limit payloads, and the ledger
//! is outside this set's territory; a `promote_model_version` row there would also have no
//! payload shape to validate against (see `ProposalKind::carries_a_payload`). The rejected
//! alternative is recorded in `docs/plans/plan-sets/set-N/phase-4-calibration-serving-ops.md`.

use std::sync::Arc;

use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::ModelManager;

/// Why a proposal could not be made or decided.
#[derive(Debug, thiserror::Error)]
pub enum PromotionError {
    #[error("version {0} of this model does not exist")]
    NoSuchVersion(i32),
    #[error("a promotion of this version to `{0}` is already pending")]
    AlreadyPending(String),
    #[error("proposal not found")]
    NotFound,
    #[error("proposal is already decided")]
    AlreadyDecided,
    #[error("a rationale is required: a decision without a basis is half an audit trail")]
    NoRationale,
    #[error("`{0}` is not a decision; use `approved` or `rejected`")]
    UnknownDecision(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn row_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "proposal_id": r.get::<Uuid, _>("proposal_id").to_string(),
        "model_id": r.get::<String, _>("model_id"),
        "version": r.get::<i32, _>("version"),
        "environment": r.get::<String, _>("environment"),
        "rationale": r.get::<String, _>("rationale"),
        "status": r.get::<String, _>("status"),
        "evidence": r.get::<Value, _>("evidence"),
        "proposed_by": r.get::<Uuid, _>("proposed_by").to_string(),
        "proposed_at": r.get::<chrono::DateTime<chrono::Utc>, _>("proposed_at").to_rfc3339(),
        "decided_by": r.get::<Option<Uuid>, _>("decided_by").map(|u| u.to_string()),
        "decided_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("decided_at").map(|t| t.to_rfc3339()),
        "decision_note": r.get::<Option<String>, _>("decision_note"),
        "performed_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("performed_at").map(|t| t.to_rfc3339()),
        "effect": r.get::<Option<Value>, _>("effect"),
    })
}

const COLS: &str = "proposal_id, model_id, version, environment, rationale, status, evidence, \
                    proposed_by, proposed_at, decided_by, decided_at, decision_note, performed_at, effect";

/// Propose promoting `version` of `model_id`. Writes nothing but the proposal.
///
/// # Errors
/// An unknown version, an already-pending proposal for the same target, an empty
/// rationale, or the database failing.
pub async fn propose(
    pg: &PgPool,
    model_id: &str,
    version: i32,
    environment: &str,
    rationale: &str,
    proposed_by: Uuid,
    evidence: &Value,
) -> Result<Uuid, PromotionError> {
    if rationale.trim().is_empty() {
        return Err(PromotionError::NoRationale);
    }
    let exists: Option<i32> = sqlx::query_scalar(
        "SELECT version FROM model_versions WHERE model_id = $1 AND version = $2",
    )
    .bind(model_id)
    .bind(version)
    .fetch_optional(pg)
    .await?;
    if exists.is_none() {
        return Err(PromotionError::NoSuchVersion(version));
    }
    let id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO model_promotion_proposals \
         (proposal_id, model_id, version, environment, rationale, evidence, proposed_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(model_id)
    .bind(version)
    .bind(environment)
    .bind(rationale)
    .bind(evidence)
    .bind(proposed_by)
    .execute(pg)
    .await;
    match res {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.constraint() == Some("model_promotion_one_pending") => {
            Err(PromotionError::AlreadyPending(environment.to_string()))
        }
        Err(e) => Err(e.into()),
    }
}

/// A model's proposals, newest first.
///
/// # Errors
/// The database failing.
pub async fn list(pg: &PgPool, model_id: &str) -> Result<Vec<Value>, PromotionError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM model_promotion_proposals WHERE model_id = $1 ORDER BY proposed_at DESC"
    ))
    .bind(model_id)
    .fetch_all(pg)
    .await?;
    Ok(rows.iter().map(row_json).collect())
}

/// The pending proposals for one version.
///
/// # Errors
/// The database failing.
pub async fn pending_for(
    pg: &PgPool,
    model_id: &str,
    version: i32,
) -> Result<Vec<Value>, PromotionError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM model_promotion_proposals \
         WHERE model_id = $1 AND version = $2 AND status = 'pending' ORDER BY proposed_at DESC"
    ))
    .bind(model_id)
    .bind(version)
    .fetch_all(pg)
    .await?;
    Ok(rows.iter().map(row_json).collect())
}

/// A person decides. `approved` performs the promotion as `decided_by`; `rejected` records
/// the decision and nothing else. The caller has already established the session is a
/// human's (the route checks the scope); this function does not re-derive authority.
///
/// # Errors
/// Unknown or already-decided proposals, an unknown decision, or the database failing.
pub async fn decide(
    manager: &Arc<ModelManager>,
    pg: &PgPool,
    proposal_id: Uuid,
    decided_by: Uuid,
    decision: &str,
    note: &str,
) -> Result<Value, PromotionError> {
    let status = match decision {
        "approved" | "rejected" => decision,
        other => return Err(PromotionError::UnknownDecision(other.to_string())),
    };
    // The decision is claimed atomically: only a still-pending row is updated, so two
    // deciders cannot both win and a decided row is never overwritten.
    let claimed = sqlx::query(&format!(
        "UPDATE model_promotion_proposals \
            SET status = $2, decided_by = $3, decided_at = now(), decision_note = $4 \
          WHERE proposal_id = $1 AND status = 'pending' \
          RETURNING {COLS}"
    ))
    .bind(proposal_id)
    .bind(status)
    .bind(decided_by)
    .bind(note)
    .fetch_optional(pg)
    .await?;
    let Some(row) = claimed else {
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT proposal_id FROM model_promotion_proposals WHERE proposal_id = $1",
        )
        .bind(proposal_id)
        .fetch_optional(pg)
        .await?;
        return Err(if exists.is_some() {
            PromotionError::AlreadyDecided
        } else {
            PromotionError::NotFound
        });
    };
    let model_id: String = row.get("model_id");
    let version: i32 = row.get("version");
    let environment: String = row.get("environment");

    if status == "rejected" {
        return Ok(row_json(&row));
    }

    // Approved: perform, as the person who decided.
    let effect = match manager
        .promote_gated(&model_id, decided_by, version, &environment, None)
        .await
    {
        Ok(v) => v,
        Err(e) => json!({"promoted": false, "reason": "error", "error": e.to_string()}),
    };
    let performed = effect.get("promoted").and_then(Value::as_bool) == Some(true);
    sqlx::query(
        "UPDATE model_promotion_proposals \
            SET effect = $2, performed_at = CASE WHEN $3 THEN now() ELSE NULL END \
          WHERE proposal_id = $1",
    )
    .bind(proposal_id)
    .bind(&effect)
    .bind(performed)
    .execute(pg)
    .await?;
    let fresh = sqlx::query(&format!(
        "SELECT {COLS} FROM model_promotion_proposals WHERE proposal_id = $1"
    ))
    .bind(proposal_id)
    .fetch_one(pg)
    .await?;
    Ok(row_json(&fresh))
}

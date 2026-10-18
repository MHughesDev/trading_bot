//! `legacy_ungated` (Set T T-1.4; ADR-P2-27, D-T3).
//!
//! Nobody is grandfathered and nothing is pulled by the gate. A deployment that
//! was running before the proposal path existed carries a marker — **a fact shown
//! beside its numbers, never a permission and never an input to any threshold** —
//! and its forward record from the day it was marked is Gate 15 evidence like any
//! other.
//!
//! Migration 0180 marks everything running at the time it applied. This marks the
//! stragglers: a deployment started between that migration and the arming of the
//! proposal requirement (`deployment_gate_switch`) has no authority and no marker,
//! and the next pass gives it the marker. Once the switch is armed there are no
//! more stragglers, because the trigger refuses an unauthorised deployment outright.
//!
//! # What reads the marker, and what cannot
//!
//! The deployments surface reads it (`deployment_gate::overview`). The review
//! worker reads the **absence of a profile pass**, never the marker. Nothing that
//! evaluates a strategy can read the table at all: `agent_role`, `backtest_role`
//! and `internal_ml_role` hold no grant on it, and a source scan in
//! `tests/deployment_path.rs` fails if a gate, ledger or evaluator source names it.

use sqlx::PgPool;

/// The basis recorded on a marker this pass writes.
pub const BASIS: &str = "running without a decided proposal when the legacy sweep found it; ADR-P2-27: nobody is grandfathered, the marker is a fact and not a permission";

/// Mark every running deployment that has neither a decided proposal behind it
/// nor a marker. Idempotent. Returns how many it marked.
///
/// # Errors
/// A database error.
pub async fn mark_unmarked(pg: &PgPool) -> Result<u64, sqlx::Error> {
    let done = sqlx::query(
        "INSERT INTO deployment_legacy_marker (deployment_id, basis)
         SELECT d.deployment_id, $1
           FROM strategy_deployment d
          WHERE d.stopped_at IS NULL
            AND d.parent_deployment_id IS NULL
            AND NOT EXISTS (SELECT 1 FROM deployment_authorisation a WHERE a.deployment_id = d.deployment_id)
            AND NOT EXISTS (SELECT 1 FROM deployment_legacy_marker m WHERE m.deployment_id = d.deployment_id)",
    )
    .bind(BASIS)
    .execute(pg)
    .await?;
    Ok(done.rows_affected())
}

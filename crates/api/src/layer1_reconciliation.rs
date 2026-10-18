//! The reader and writer of `strategy_reconciliation` (Set M M-4.1, M-4.4; migration 0205).
//!
//! The comparator (`backtest::layer_reconciliation::reconcile`) produces a verdict and the table
//! holds it append-only; this is the code that moves one into the other and the other back out
//! as the records Gate 3 reads (`backtest::gates::reconciliation_gate`). Before it, the table was
//! written by nobody and read by nobody.
//!
//! The table is not tenant-scoped (a revision hash is content, the same for everyone), so these
//! queries take no tenant. The CHECK constraints in the migration are the real validation: a
//! `passed` row that contradicts its own thresholds is refused by the database whatever this code
//! says, and the writer maps that refusal to an error rather than retrying it.

use backtest::gates::reconciliation_gate::ReconRecord;
use backtest::layer_reconciliation::Reconciliation;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Record a verdict. The same pair, experiment and thresholds is one verdict (a UNIQUE
/// constraint), so recording it twice is an error the caller sees, not a second row.
///
/// # Errors
/// The database refusing the row: an unknown revision, an inconsistent `passed`, a repeat.
pub async fn record(
    pg: &PgPool,
    layer1_revision: &str,
    layer2_revision: &str,
    experiment_id: &str,
    r: &Reconciliation,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO strategy_reconciliation \
         (layer1_revision, layer2_revision, experiment_id, n_decisions, agreement, pnl_tracking_error, \
          thresholds, explained_diffs, unexplained_count, passed) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING reconciliation_id",
    )
    .bind(layer1_revision)
    .bind(layer2_revision)
    .bind(experiment_id)
    .bind(i32::try_from(r.n_decisions).unwrap_or(i32::MAX))
    .bind(r.agreement)
    .bind(r.pnl_tracking_error)
    .bind(json!({ "agreement_min": r.thresholds.agreement_min, "te_max": r.thresholds.te_max }))
    .bind(serde_json::to_value(&r.disagreements).unwrap_or_else(|_| json!([])))
    .bind(i32::try_from(r.unexplained).unwrap_or(i32::MAX))
    .bind(r.passed)
    .fetch_one(pg)
    .await
}

/// Every record for a Layer 2 revision, as the gate reads them.
///
/// # Errors
/// A backend failure.
pub async fn records_for(pg: &PgPool, layer2_revision: &str) -> Result<Vec<ReconRecord>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT layer1_revision, layer2_revision, passed, created_at FROM strategy_reconciliation \
         WHERE layer2_revision = $1 ORDER BY created_at",
    )
    .bind(layer2_revision)
    .fetch_all(pg)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(ReconRecord {
                layer1_revision: r.try_get("layer1_revision")?,
                layer2_revision: r.try_get("layer2_revision")?,
                passed: r.try_get("passed")?,
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

/// The requirement a Layer 2 revision owes at Gate 3, given its recorded parent.
///
/// # Errors
/// A backend failure reading the records.
pub async fn requirement_for(
    pg: &PgPool,
    layer2_revision: &str,
    layer1_parent: &str,
) -> Result<backtest::gates::reconciliation_gate::ReconciliationRequirement, sqlx::Error> {
    Ok(backtest::gates::reconciliation_gate::ReconciliationRequirement {
        layer2_revision: layer2_revision.to_string(),
        layer1_parent: layer1_parent.to_string(),
        records: records_for(pg, layer2_revision).await?,
    })
}

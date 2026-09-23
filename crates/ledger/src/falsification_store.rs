//! Persistence for falsification reports (migration 0059).
//!
//! Separate from the types in [`crate::falsification`] so those stay usable
//! without a database: the constructors are the enforcement, and a test that
//! exercises them should not need Postgres to do it.
//!
//! The database enforces the same rules a second time. That is not redundancy —
//! a direct SQL client bypasses the Rust type and does not bypass a CHECK
//! constraint, and the agent is not the only thing that can write here.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::falsification::{
    EvaluatorDefect, FalsificationError, Kill, KillClass, Reproduction, Score, Verification,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Invalid(#[from] FalsificationError),
    #[error("database: {0}")]
    Backend(String),
}

fn be(e: &sqlx::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// The closed vocabulary, paired with the spellings migration 0059's CHECK
/// constraint accepts.
///
/// One table rather than two `match` arms: a class whose code does not match the
/// constraint fails on INSERT, at 3 a.m., on the one report that mattered, and
/// `every_kill_class_round_trips_through_its_database_code` is what stops that.
const KILL_CLASSES: [(KillClass, &str); 7] = [
    (KillClass::Lookahead, "lookahead"),
    (KillClass::DataArtifact, "data_artifact"),
    (KillClass::BelowCostFloor, "below_cost_floor"),
    (
        KillClass::BelowTradeableFrontier,
        "below_tradeable_frontier",
    ),
    (KillClass::FillModelExploit, "fill_model_exploit"),
    (KillClass::DuplicateOfExisting, "duplicate_of_existing"),
    (KillClass::InvalidMetric, "invalid_metric"),
];

/// The database spelling of a kill class.
#[must_use]
pub fn class_code(c: KillClass) -> &'static str {
    KILL_CLASSES
        .iter()
        .find(|(k, _)| *k == c)
        .map_or("invalid_metric", |(_, s)| *s)
}

/// Parse a class code coming back from the database.
#[must_use]
pub fn class_from_code(s: &str) -> Option<KillClass> {
    KILL_CLASSES
        .iter()
        .find(|(_, code)| *code == s)
        .map(|(k, _)| *k)
}

fn verification_code(v: Verification) -> &'static str {
    match v {
        Verification::Unverified => "unverified",
        Verification::Verified => "verified",
        Verification::Refuted => "refuted",
    }
}

/// File a kill.
///
/// # Errors
/// Backend failures, including the CHECK constraints that refuse an
/// unfalsifiable claim.
pub async fn insert_kill(
    pool: &PgPool,
    tenant_id: &str,
    reporter: (&str, &str),
    target_trial_id: Uuid,
    kill: &Kill,
) -> Result<Uuid, StoreError> {
    let r: &Reproduction = kill.reproduction();
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, target_trial_id, defect_class,
             artifact_ref, seed, expected, observed, steps,
             verification, reported_by, reporter_kind
         ) VALUES ($1,'kill',$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
         RETURNING falsification_id",
    )
    .bind(tenant_id)
    .bind(target_trial_id)
    .bind(class_code(kill.defect()))
    .bind(r.artifact_ref())
    .bind(i64::try_from(r.seed()).unwrap_or(i64::MAX))
    .bind(r.expected())
    .bind(r.observed())
    .bind(r.steps())
    .bind(verification_code(kill.verification()))
    .bind(reporter.0)
    .bind(reporter.1)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| be(&e))?;
    tx.commit().await.map_err(|e| be(&e))?;
    Ok(id)
}

/// File an evaluator defect.
///
/// # Errors
/// Backend failures.
pub async fn insert_defect(
    pool: &PgPool,
    tenant_id: &str,
    reporter: (&str, &str),
    defect: &EvaluatorDefect,
) -> Result<Uuid, StoreError> {
    let r: &Reproduction = defect.reproduction();
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, component, claimed_guarantee,
             artifact_ref, seed, expected, observed, steps,
             verification, regression_test, reported_by, reporter_kind
         ) VALUES ($1,'evaluator_defect',$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         RETURNING falsification_id",
    )
    .bind(tenant_id)
    .bind(defect.component())
    .bind(defect.claimed_guarantee())
    .bind(r.artifact_ref())
    .bind(i64::try_from(r.seed()).unwrap_or(i64::MAX))
    .bind(r.expected())
    .bind(r.observed())
    .bind(r.steps())
    .bind(verification_code(defect.verification()))
    .bind(defect.regression_test())
    .bind(reporter.0)
    .bind(reporter.1)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| be(&e))?;
    tx.commit().await.map_err(|e| be(&e))?;
    Ok(id)
}

/// Record a verification verdict.
///
/// The claim itself is immutable — 0059's trigger refuses an UPDATE touching
/// anything but these columns — so this cannot be used to edit a report after
/// somebody tried to reproduce it.
///
/// # Errors
/// Backend failures.
pub async fn verify(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    outcome: Verification,
    verified_by: &str,
) -> Result<bool, StoreError> {
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let n = sqlx::query(
        "UPDATE mlops.falsification
            SET verification = $1, verified_by = $2, verified_at = now()
          WHERE falsification_id = $3 AND tenant_id = $4",
    )
    .bind(verification_code(outcome))
    .bind(verified_by)
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| be(&e))?
    .rows_affected();
    tx.commit().await.map_err(|e| be(&e))?;
    Ok(n == 1)
}

/// Attach the regression test that discharges a defect.
///
/// # Errors
/// Backend failures.
pub async fn covered_by(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    test: &str,
) -> Result<bool, StoreError> {
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let n = sqlx::query(
        "UPDATE mlops.falsification SET regression_test = $1
          WHERE falsification_id = $2 AND tenant_id = $3",
    )
    .bind(test)
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| be(&e))?
    .rows_affected();
    tx.commit().await.map_err(|e| be(&e))?;
    Ok(n == 1)
}

/// The score the database assigns one report, read through
/// `mlops.falsification_score` rather than recomputed here.
///
/// Reading it back rather than computing it is the point: if the SQL function
/// and [`Score`] ever disagree, this is where it shows.
///
/// # Errors
/// Backend failures.
pub async fn score_of(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Option<Score>, StoreError> {
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let row = sqlx::query(
        "SELECT score FROM mlops.falsification_scored
          WHERE falsification_id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| be(&e))?;
    tx.commit().await.map_err(|e| be(&e))?;

    Ok(row.map(|r| match r.get::<i32, _>("score") {
        3 => Score::EvaluatorDefect,
        2 => Score::Promotion,
        1 => Score::Kill,
        _ => Score::Zero,
    }))
}

/// Verified defects with no regression test: the work queue.
///
/// # Errors
/// Backend failures.
pub async fn undischarged(pool: &PgPool, tenant_id: &str) -> Result<Vec<Uuid>, StoreError> {
    let mut tx = crate::pg::tenant_tx(pool, tenant_id)
        .await
        .map_err(|e| be(&e))?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT falsification_id FROM mlops.falsification
          WHERE tenant_id = $1 AND report_kind = 'evaluator_defect'
            AND verification = 'verified' AND regression_test IS NULL
          ORDER BY reported_at",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| be(&e))?;
    tx.commit().await.map_err(|e| be(&e))?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kill_class_round_trips_through_its_database_code() {
        for (class, code) in KILL_CLASSES {
            assert_eq!(class_code(class), code);
            assert_eq!(class_from_code(code), Some(class));
        }
    }

    /// There is no code for a performance threshold, and a lookup for one must
    /// not quietly resolve to something else.
    #[test]
    fn there_is_no_code_for_underperformance() {
        assert_eq!(class_from_code("underperforms"), None);
        assert_eq!(class_from_code("low_sharpe"), None);
        assert_eq!(class_from_code(""), None);
    }

    #[test]
    fn verification_codes_match_the_check_constraint() {
        assert_eq!(verification_code(Verification::Unverified), "unverified");
        assert_eq!(verification_code(Verification::Verified), "verified");
        assert_eq!(verification_code(Verification::Refuted), "refuted");
    }
}

//! Persistence for the forward record (migration 0105).
//!
//! Runtime-bound queries rather than `sqlx::query!`, matching `crates/ledger`:
//! the macro form would make every build of this crate depend on a reachable
//! database, and reconciliation has to compile on a machine that has never seen
//! one.
//!
//! Every write here is an append. `mlops.forward_observation`,
//! `mlops.divergence_halt` and `mlops.divergence_halt_clearance` all carry
//! `refuse_mutation` triggers, so a corrected observation is a new row and the
//! history of what was believed stays legible.

use chrono::{DateTime, Utc};
use domain::forward::{ForwardObservation, ObservationSource};
use domain::numeraire::Numeraire;
use risk::{DivergenceHalt, DivergenceKind, HaltClearance};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Append one forward observation.
///
/// `ON CONFLICT DO NOTHING` against the `(deployment_id, bar, source)` unique
/// key: a replayed bar close is a no-op rather than a double-count, and a
/// double-counted forward observation inflates exactly the number that is
/// hardest to acquire.
///
/// # Errors
/// Any database error.
pub async fn insert_observation(
    pool: &PgPool,
    tenant_id: &str,
    o: &ForwardObservation,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mlops.forward_observation \
         (tenant_id, deployment_id, strategy_id, bar, realised_return, expected_return, \
          numeraire, source, signal_matched, realised_slippage_bps, modelled_slippage_bps, \
          realised_turnover, expected_turnover, model_attributable_rejects, observed_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) \
         ON CONFLICT (deployment_id, bar, source) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(o.deployment_id)
    .bind(&o.strategy_id)
    .bind(o.bar)
    .bind(o.realised_return)
    .bind(o.expected_return)
    // Narrowed here, with a named error, rather than left to migration 0105's
    // CHECK: the domain type is deliberately wider than the six tokens the
    // schema accepts, and a row that cannot be stored should say so before it
    // is built, not after Postgres refuses it.
    .bind(
        o.numeraire
            .as_storage_token()
            .map_err(|e| sqlx::Error::Encode(Box::new(e)))?,
    )
    .bind(o.source.as_str())
    .bind(o.signal_matched)
    .bind(o.realised_slippage_bps)
    .bind(o.modelled_slippage_bps)
    .bind(o.realised_turnover)
    .bind(o.expected_turnover)
    .bind(i32::try_from(o.model_attributable_rejects).unwrap_or(i32::MAX))
    .bind(o.observed_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Every observation for a deployment in `[from, to]`, oldest first.
///
/// Returns **both** sources deliberately. The observed-knowledge filter belongs
/// in one place — `Gate15Evidence::from_observations` — and a reader that
/// filtered here as well would give the rule two homes and two chances to drift.
/// It also means the caller can report how many backfilled rows were excluded,
/// which is the difference between "no evidence" and "no evidence, and here is
/// what was in the window instead".
///
/// # Errors
/// Any database error, and an unparseable `source` or `numeraire` — which the
/// table's CHECKs make unreachable, and which is surfaced rather than defaulted
/// because defaulting an unknown source to `observed` is how a backfilled row
/// becomes forward evidence.
pub async fn load_observations(
    pool: &PgPool,
    tenant_id: &str,
    deployment_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<ForwardObservation>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT deployment_id, strategy_id, bar, realised_return, expected_return, numeraire, \
                source, signal_matched, realised_slippage_bps, modelled_slippage_bps, \
                realised_turnover, expected_turnover, model_attributable_rejects, observed_at \
           FROM mlops.forward_observation \
          WHERE tenant_id = $1 AND deployment_id = $2 AND bar >= $3 AND bar <= $4 \
          ORDER BY bar ASC",
    )
    .bind(tenant_id)
    .bind(deployment_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let source: String = r.try_get("source")?;
        let numeraire: String = r.try_get("numeraire")?;
        let rejects: i32 = r.try_get("model_attributable_rejects")?;
        out.push(ForwardObservation {
            deployment_id: r.try_get("deployment_id")?,
            strategy_id: r.try_get("strategy_id")?,
            bar: r.try_get("bar")?,
            realised_return: r.try_get("realised_return")?,
            expected_return: r.try_get("expected_return")?,
            numeraire: numeraire
                .parse::<Numeraire>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            source: source
                .parse::<ObservationSource>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            signal_matched: r.try_get("signal_matched")?,
            realised_slippage_bps: r.try_get("realised_slippage_bps")?,
            modelled_slippage_bps: r.try_get("modelled_slippage_bps")?,
            realised_turnover: r.try_get("realised_turnover")?,
            expected_turnover: r.try_get("expected_turnover")?,
            model_attributable_rejects: u32::try_from(rejects).unwrap_or(0),
            observed_at: r.try_get("observed_at")?,
        });
    }
    Ok(out)
}

/// Write a raised halt. Called after the in-memory registry already stopped the
/// strategy — durability follows the stop, never gates it, because a halt that
/// waited on a database would fail *open* whenever the database was slow.
///
/// # Errors
/// Any database error.
pub async fn insert_halt(
    pool: &PgPool,
    tenant_id: &str,
    halt: &DivergenceHalt,
    deployment_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mlops.divergence_halt \
         (halt_id, tenant_id, strategy_id, deployment_id, kind, reason, raised_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (halt_id) DO NOTHING",
    )
    .bind(halt.halt_id)
    .bind(tenant_id)
    .bind(&halt.strategy_id)
    .bind(deployment_id)
    .bind(halt.kind.as_str())
    .bind(&halt.reason)
    .bind(halt.raised_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Record the human act of clearing a halt. The primary key is the halt's own
/// id, so a second clearance is a constraint violation rather than a second row
/// nobody notices.
///
/// # Errors
/// Any database error.
pub async fn insert_clearance(
    pool: &PgPool,
    tenant_id: &str,
    clearance: &HaltClearance,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mlops.divergence_halt_clearance \
         (halt_id, tenant_id, cleared_by, note, cleared_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(clearance.halt_id)
    .bind(tenant_id)
    .bind(clearance.cleared_by.as_str())
    .bind(&clearance.note)
    .bind(clearance.cleared_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Halts with no clearance — what the registry must hold after a restart.
///
/// Without this the in-memory registry would come up empty on every boot, which
/// is the failure mode of any in-memory safety flag and the one that looks like
/// the system deciding on its own that the divergence is over.
///
/// # Errors
/// Any database error, or a `kind` the table's CHECK should have made
/// impossible.
pub async fn load_active_halts(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<DivergenceHalt>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT h.halt_id, h.strategy_id, h.kind, h.reason, h.raised_at \
           FROM mlops.divergence_halt h \
           LEFT JOIN mlops.divergence_halt_clearance c ON c.halt_id = h.halt_id \
          WHERE h.tenant_id = $1 AND c.halt_id IS NULL \
          ORDER BY h.raised_at ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let kind: String = r.try_get("kind")?;
        let kind = match kind.as_str() {
            "position" => DivergenceKind::Position,
            "return" => DivergenceKind::Return,
            "balance" => DivergenceKind::Balance,
            other => {
                return Err(sqlx::Error::Decode(
                    format!("unknown divergence kind {other:?}").into(),
                ))
            }
        };
        out.push(DivergenceHalt {
            halt_id: r.try_get("halt_id")?,
            strategy_id: r.try_get("strategy_id")?,
            kind,
            reason: r.try_get("reason")?,
            raised_at: r.try_get("raised_at")?,
        });
    }
    Ok(out)
}

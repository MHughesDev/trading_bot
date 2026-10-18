//! Divergence halts across a restart, and the human act that clears one
//! (Set T T-0.3; SC-6, FM-4).
//!
//! `risk::StrategyHaltRegistry` is in memory, inside the order gate, which is
//! why a halt stops the next order with no database in the way. The cost is the
//! failure mode of every in-memory safety flag: a restart forgets it, and that
//! looks exactly like the platform deciding on its own that the divergence is
//! over. `mlops.divergence_halt` is the durable half and this module is what
//! carries it back into the gate: [`rehydrate`] on every cadence, from the
//! table, for every tenant.
//!
//! Clearing goes the other way and is only ever a person: [`clear`] takes a
//! [`HumanActor`] — which cannot be built empty — and a non-empty note, lifts the
//! in-memory halt and writes the `mlops.divergence_halt_clearance` row. If the
//! write fails the next [`rehydrate`] re-imposes the halt, because the durable
//! halt has no clearance: the failure direction is "still halted".

use std::sync::Arc;
use std::time::Duration;

use risk::{ClearError, DivergenceHalt, HaltClearance, HumanActor, StrategyHaltRegistry};
use sqlx::PgPool;

/// How often the registry is re-read from the table.
pub const REHYDRATE_SECS: u64 = 30;

/// Why a clearance failed.
#[derive(Debug, thiserror::Error)]
pub enum ClearHaltError {
    #[error(transparent)]
    Refused(#[from] ClearError),
    /// The halt was lifted in memory and the clearance could not be written.
    /// The next rehydration re-imposes it.
    #[error("the clearance could not be recorded, so the halt will be re-imposed: {0}")]
    NotRecorded(#[from] sqlx::Error),
}

/// Every durable halt, for one tenant, pushed into the registry.
///
/// Idempotent: `rehydrate` keeps the first halt for a strategy, so re-reading
/// the same rows every cadence changes nothing.
///
/// # Errors
/// The database.
pub async fn rehydrate(
    pool: &PgPool,
    tenant_id: &str,
    registry: &StrategyHaltRegistry,
) -> Result<usize, sqlx::Error> {
    let halts = crate::store::load_active_halts(pool, tenant_id).await?;
    let n = halts.len();
    for h in halts {
        registry.rehydrate(h);
    }
    Ok(n)
}

/// Every halt a tenant has in force: the registry's, plus durable ones the
/// registry does not hold yet. The UI surface.
///
/// # Errors
/// The database.
pub async fn active(
    pool: &PgPool,
    tenant_id: &str,
    registry: &StrategyHaltRegistry,
) -> Result<Vec<(DivergenceHalt, bool)>, sqlx::Error> {
    let durable = crate::store::load_active_halts(pool, tenant_id).await?;
    let mut out: Vec<(DivergenceHalt, bool)> = registry
        .active()
        .into_iter()
        .map(|h| {
            let persisted = durable.iter().any(|d| d.halt_id == h.halt_id);
            (h, persisted)
        })
        .collect();
    for d in durable {
        if !out
            .iter()
            .any(|(h, _)| h.halt_id == d.halt_id || h.strategy_id == d.strategy_id)
        {
            // Durable and not yet in force in this process: rehydrate it now
            // rather than show a halt the gate is not enforcing.
            registry.rehydrate(d.clone());
            out.push((d, true));
        }
    }
    out.sort_by_key(|(h, _)| h.raised_at);
    Ok(out)
}

/// Lift one strategy's halt as a named human, and record it.
///
/// # Errors
/// [`ClearHaltError::Refused`] for an empty note or a strategy that is not
/// halted; [`ClearHaltError::NotRecorded`] if the clearance could not be
/// written.
pub async fn clear(
    pool: &PgPool,
    tenant_id: &str,
    registry: &StrategyHaltRegistry,
    strategy_id: &str,
    by: HumanActor,
    note: &str,
) -> Result<HaltClearance, ClearHaltError> {
    // A halt that is durable but not yet rehydrated is still a halt.
    rehydrate(pool, tenant_id, registry).await?;
    let clearance = registry.clear(strategy_id, by, note)?;
    crate::store::insert_clearance(pool, tenant_id, &clearance).await?;
    Ok(clearance)
}

/// Re-read every tenant's durable halts into the gate's registry, forever.
///
/// The caller is the process that owns the order gate; see the shared-file
/// request in the Set T phase-0 notes for the one `main.rs` line that spawns it.
pub fn spawn_rehydration(pool: PgPool, registry: Arc<StrategyHaltRegistry>) {
    tokio::spawn(async move {
        let ledger = ledger::pg::PgTrialLedger::new(pool.clone());
        let mut tick = tokio::time::interval(Duration::from_secs(REHYDRATE_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let tenants = match ledger.tenants_async().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, "divergence halts not rehydrated: tenants unreadable");
                    continue;
                }
            };
            for t in tenants {
                if let Err(e) = rehydrate(&pool, &t, &registry).await {
                    tracing::error!(tenant = %t, error = %e, "divergence halts not rehydrated");
                }
            }
        }
    });
}

//! Position reconciliation: compare internal state against a venue.
//!
//! A sweep is **per venue**, because the fact it can establish is per venue:
//! "this venue and our book disagree about what is owned". One broker is one
//! venue, and the halt that follows names it.
//!
//! ## Where this runs
//!
//! Nowhere, yet. `apps/platform` declares the `reconciliation` dependency and no
//! file under `apps/platform/src/` mentions the crate; the 30-second sweep loop
//! an earlier version of this module doc described has never existed. It is not
//! wired because the platform builds exactly one broker —
//! `PaperTradingEngine::multi_asset_broker()`, whose `query_positions` returns
//! `engine.positions(class)` — and the internal book it would be compared
//! against is the same value from the same `Arc`. A sweep over that pair cannot
//! diverge, and a reconciliation that cannot fail is a verdict of "reconciled"
//! that nothing measured.
//!
//! The loop belongs next to the first broker that talks to something outside
//! this process.

use std::sync::Arc;

use rust_decimal::Decimal;

use execution::broker::{Broker, BrokerError, BrokerPosition};
use monitor::store::{MonitorStore, StoreError};
use risk::KillSwitch;

use crate::divergence::{compare_position, ReconcileOutcome};

/// A single internal position snapshot to reconcile.
#[derive(Debug, Clone)]
pub struct InternalPosition {
    pub instrument_id: String,
    pub quantity: Decimal,
}

/// A sweep that could not reach a verdict.
///
/// Neither variant means "reconciled". A venue whose positions could not be
/// read has not been checked, and a mismatch whose halt could not be recorded
/// has not been halted — both are absences, and the type is what stops a caller
/// spelling either of them as a clean result.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// The venue's positions could not be read.
    #[error("the venue's positions could not be read: {0}")]
    Unreadable(#[from] BrokerError),
    /// The mismatch could not be recorded, so it was not halted.
    #[error("{0}")]
    NotHalted(#[from] StoreError),
}

/// Compare one internal position against what the venue reports. Raises nothing.
///
/// A venue that does not list an instrument is reporting **flat** in it — that
/// is what an omitted position means in every adapter here, not an unread
/// measurement, which arrives as an error from `query_positions` instead and
/// never reaches this function.
#[must_use]
pub fn compare_one(
    internal: &InternalPosition,
    broker_positions: &[BrokerPosition],
) -> ReconcileOutcome {
    // Pre-compute the normalized (no-dash) form once rather than per-comparison.
    let normalized_id = internal.instrument_id.replace('-', "");
    let broker_qty = broker_positions
        .iter()
        .find(|p| p.instrument_id == internal.instrument_id || p.instrument_id == normalized_id)
        .map_or(Decimal::ZERO, |p| p.quantity);

    compare_position(&internal.instrument_id, internal.quantity, broker_qty)
}

/// Compare every internal position against what the venue reports. Raises
/// nothing.
#[must_use]
pub fn compare_all(
    internal_positions: &[InternalPosition],
    broker_positions: &[BrokerPosition],
) -> Vec<ReconcileOutcome> {
    internal_positions
        .iter()
        .map(|ip| compare_one(ip, broker_positions))
        .collect()
}

/// Reconcile a single instrument's internal position against `venue_id`.
///
/// Halts `venue:<venue_id>` durably on divergence, through
/// [`check_position_divergence_at_venue`][c].
///
/// # Errors
/// [`SweepError::NotHalted`] if the mismatch could not be recorded.
///
/// [c]: crate::divergence::check_position_divergence_at_venue
pub async fn reconcile_one(
    internal: &InternalPosition,
    venue_id: &str,
    broker_positions: &[BrokerPosition],
    store: &MonitorStore,
    kill_switch: &Arc<KillSwitch>,
) -> Result<ReconcileOutcome, SweepError> {
    Ok(reconcile_all(
        std::slice::from_ref(internal),
        venue_id,
        broker_positions,
        store,
        kill_switch,
    )
    .await?
    .pop()
    .expect("one position in, one outcome out"))
}

/// Reconcile all internal positions against one venue's positions.
///
/// Returns an outcome per instrument. If **any** of them diverged the venue is
/// halted once, before this returns — once, and not once per instrument,
/// because one sweep that disagrees is one incident and `monitor.events` is the
/// timeline an operator reads. Every diverging instrument is logged.
///
/// # Errors
/// [`SweepError::NotHalted`] if the mismatch could not be recorded. The
/// outcomes are then not returned at all: a caller holding a `Diverged` it
/// believes was halted is the failure this ordering exists to prevent.
pub async fn reconcile_all(
    internal_positions: &[InternalPosition],
    venue_id: &str,
    broker_positions: &[BrokerPosition],
    store: &MonitorStore,
    kill_switch: &Arc<KillSwitch>,
) -> Result<Vec<ReconcileOutcome>, SweepError> {
    let outcomes = compare_all(internal_positions, broker_positions);

    let diverged: Vec<&ReconcileOutcome> = outcomes
        .iter()
        .filter(|o| matches!(o, ReconcileOutcome::Diverged { .. }))
        .collect();

    if !diverged.is_empty() {
        for outcome in &diverged {
            if let ReconcileOutcome::Diverged {
                instrument_id,
                internal_qty,
                broker_qty,
            } = outcome
            {
                tracing::error!(
                    %venue_id,
                    %instrument_id,
                    %internal_qty,
                    %broker_qty,
                    "POSITION DIVERGENCE — halting this venue"
                );
            }
        }
        let halted = store.carry_out_mismatch(venue_id, kill_switch).await?;
        tracing::warn!(scope = %halted, instruments = diverged.len(), "venue halted and recorded");
    }

    Ok(outcomes)
}

/// Fetch one venue's positions and reconcile the internal book against them.
///
/// # Errors
/// [`SweepError::Unreadable`] if the venue's positions could not be read —
/// which is not "reconciled" and must never be recorded as one — or
/// [`SweepError::NotHalted`] if a mismatch could not be recorded.
pub async fn reconcile_with_broker(
    internal_positions: &[InternalPosition],
    venue_id: &str,
    broker: &dyn Broker,
    store: &MonitorStore,
    kill_switch: &Arc<KillSwitch>,
) -> Result<Vec<ReconcileOutcome>, SweepError> {
    let broker_positions = broker.query_positions().await?;
    reconcile_all(
        internal_positions,
        venue_id,
        &broker_positions,
        store,
        kill_switch,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Price;
    use std::str::FromStr;

    fn broker_pos(instrument_id: &str, qty: &str) -> BrokerPosition {
        BrokerPosition {
            instrument_id: instrument_id.to_owned(),
            quantity: Decimal::from_str(qty).unwrap(),
            avg_entry_price: Price::from_str("1").unwrap(),
        }
    }

    fn internal(instrument_id: &str, qty: &str) -> InternalPosition {
        InternalPosition {
            instrument_id: instrument_id.to_owned(),
            quantity: Decimal::from_str(qty).unwrap(),
        }
    }

    // The halting half needs a database and is tested in
    // `tests/a_venue_mismatch_survives_the_mirror_refresh.rs`. These cover the
    // comparison, which is the part that decides whether a halt happens at all.

    #[test]
    fn matching_positions_ok() {
        let broker = vec![broker_pos("BTC-USD", "1.0")];
        assert_eq!(
            compare_one(&internal("BTC-USD", "1.0"), &broker),
            ReconcileOutcome::Match
        );
    }

    #[test]
    fn a_venue_that_does_not_list_an_instrument_is_flat_in_it() {
        // Flat against flat agrees; holding something the venue does not list is
        // a disagreement about what is owned, which is the whole point.
        assert_eq!(
            compare_one(&internal("BTC-USD", "0"), &[]),
            ReconcileOutcome::Match
        );
        assert!(matches!(
            compare_one(&internal("BTC-USD", "1.0"), &[]),
            ReconcileOutcome::Diverged { .. }
        ));
    }

    #[test]
    fn the_dashless_spelling_is_the_same_instrument() {
        let broker = vec![broker_pos("BTCUSD", "1.0")];
        assert_eq!(
            compare_one(&internal("BTC-USD", "1.0"), &broker),
            ReconcileOutcome::Match
        );
    }
}

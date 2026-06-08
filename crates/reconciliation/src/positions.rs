//! Position reconciliation: compare internal state against broker.
//!
//! Runs:
//! - On every fill (triggered by `ExecutionEngine`).
//! - In a 30-second sweep loop (driven by the platform app).
//! - On every reconnect (before resuming trading).

use std::sync::Arc;

use rust_decimal::Decimal;

use execution::broker::{Broker, BrokerPosition};
use risk::KillSwitch;

use crate::divergence::{check_position_divergence, ReconcileOutcome};

/// A single internal position snapshot to reconcile.
#[derive(Debug, Clone)]
pub struct InternalPosition {
    pub instrument_id: String,
    pub quantity: Decimal,
}

/// Reconcile a single instrument's internal position against the broker.
///
/// Returns `ReconcileOutcome`.  Trips the kill switch on divergence.
pub fn reconcile_one(
    internal: &InternalPosition,
    broker_positions: &[BrokerPosition],
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    let broker_qty = broker_positions
        .iter()
        .find(|p| {
            p.instrument_id == internal.instrument_id
                || p.instrument_id == internal.instrument_id.replace('-', "")
        })
        .map(|p| p.quantity)
        .unwrap_or(Decimal::ZERO);

    check_position_divergence(
        &internal.instrument_id,
        internal.quantity,
        broker_qty,
        kill_switch,
    )
}

/// Reconcile all internal positions against broker positions.
///
/// Returns a list of outcomes for each instrument checked.
pub fn reconcile_all(
    internal_positions: &[InternalPosition],
    broker_positions: &[BrokerPosition],
    kill_switch: &Arc<KillSwitch>,
) -> Vec<ReconcileOutcome> {
    internal_positions
        .iter()
        .map(|ip| reconcile_one(ip, broker_positions, kill_switch))
        .collect()
}

/// Fetch broker positions and reconcile against internal state.
pub async fn reconcile_with_broker(
    internal_positions: &[InternalPosition],
    broker: &dyn Broker,
    kill_switch: &Arc<KillSwitch>,
) -> Result<Vec<ReconcileOutcome>, execution::broker::BrokerError> {
    let broker_positions = broker.query_positions().await?;
    Ok(reconcile_all(
        internal_positions,
        &broker_positions,
        kill_switch,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Price;
    use std::str::FromStr;

    fn ks() -> Arc<KillSwitch> {
        Arc::new(KillSwitch::new(false))
    }

    fn broker_pos(instrument_id: &str, qty: &str) -> BrokerPosition {
        BrokerPosition {
            instrument_id: instrument_id.to_owned(),
            quantity: Decimal::from_str(qty).unwrap(),
            avg_entry_price: Price::from_str("1").unwrap(),
        }
    }

    #[test]
    fn matching_positions_ok() {
        let ks = ks();
        let internal = InternalPosition {
            instrument_id: "BTC-USD".to_owned(),
            quantity: Decimal::from_str("1.0").unwrap(),
        };
        let broker = vec![broker_pos("BTC-USD", "1.0")];
        let outcome = reconcile_one(&internal, &broker, &ks);
        assert_eq!(outcome, ReconcileOutcome::Match);
        assert!(!ks.is_active());
    }

    #[test]
    fn missing_broker_position_treated_as_zero() {
        let ks = ks();
        let internal = InternalPosition {
            instrument_id: "BTC-USD".to_owned(),
            quantity: Decimal::from_str("1.0").unwrap(),
        };
        let outcome = reconcile_one(&internal, &[], &ks); // no broker positions
        assert!(matches!(outcome, ReconcileOutcome::Diverged { .. }));
        assert!(ks.is_active());
    }
}

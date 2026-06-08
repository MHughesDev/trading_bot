//! Divergence detection between internal position view and broker position.
//!
//! On divergence: trips the kill switch for the affected instrument and
//! logs an alarm.  Does **not** force-close the position.

use std::sync::Arc;

use rust_decimal::Decimal;
use tracing::{error, warn};

use risk::KillSwitch;

/// Tolerance within which positions are considered equal (rounding noise).
const QUANTITY_TOLERANCE: &str = "0.000001";

/// Outcome of a position reconciliation check.
#[derive(Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Internal and broker positions match within tolerance.
    Match,
    /// Positions diverge; kill switch tripped for this instrument.
    Diverged {
        instrument_id: String,
        internal_qty: String,
        broker_qty: String,
    },
}

/// Compare `internal_qty` against `broker_qty`.  If they diverge beyond
/// tolerance, trip the kill switch and return `Diverged`.
pub fn check_position_divergence(
    instrument_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    let tolerance: Decimal = QUANTITY_TOLERANCE.parse().expect("constant is valid");
    let delta = (internal_qty - broker_qty).abs();

    if delta > tolerance {
        error!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            %delta,
            "POSITION DIVERGENCE — tripping kill switch"
        );
        kill_switch.trip();
        ReconcileOutcome::Diverged {
            instrument_id: instrument_id.to_owned(),
            internal_qty: internal_qty.to_string(),
            broker_qty: broker_qty.to_string(),
        }
    } else {
        warn!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            "position reconciliation OK"
        );
        ReconcileOutcome::Match
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn ks() -> Arc<KillSwitch> {
        Arc::new(KillSwitch::new(false))
    }

    #[test]
    fn matching_positions_return_ok() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("1.0").unwrap(),
            &ks,
        );
        assert_eq!(result, ReconcileOutcome::Match);
        assert!(!ks.is_active());
    }

    #[test]
    fn divergent_positions_trip_kill_switch() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("2.0").unwrap(), // divergence of 1.0
            &ks,
        );
        assert!(matches!(result, ReconcileOutcome::Diverged { .. }));
        assert!(ks.is_active(), "kill switch must be tripped on divergence");
    }

    #[test]
    fn tiny_rounding_noise_within_tolerance_is_ignored() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("1.0000005").unwrap(), // sub-tolerance
            &ks,
        );
        assert_eq!(result, ReconcileOutcome::Match);
        assert!(!ks.is_active());
    }
}

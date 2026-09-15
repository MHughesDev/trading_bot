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
///
/// Trips the **global** scope, because this signature does not know which venue
/// the divergence is at. Prefer [`check_position_divergence_at_venue`] wherever
/// the venue is known: COMP-007 §4 halts the venue that disagrees and leaves the
/// others trading, and a global halt for one venue's bookkeeping is the kind of
/// over-reaction that gets a safety mechanism switched off.
pub fn check_position_divergence(
    instrument_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    check_divergence(instrument_id, None, internal_qty, broker_qty, kill_switch)
}

/// The venue-scoped form (COMP-007 §4, §8.3).
///
/// Halts `venue:<id>` and nothing else. The other venues keep trading, because
/// the fact established here is "this venue and our book disagree about what is
/// owned", which says nothing at all about the others.
pub fn check_position_divergence_at_venue(
    instrument_id: &str,
    venue_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    check_divergence(
        instrument_id,
        Some(venue_id),
        internal_qty,
        broker_qty,
        kill_switch,
    )
}

fn check_divergence(
    instrument_id: &str,
    venue_id: Option<&str>,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    let tolerance: Decimal = QUANTITY_TOLERANCE.parse().expect("constant is valid");
    let delta = (internal_qty - broker_qty).abs();

    if delta > tolerance {
        error!(
            %instrument_id,
            venue_id = venue_id.unwrap_or("<unknown>"),
            %internal_qty,
            %broker_qty,
            %delta,
            "POSITION DIVERGENCE — tripping kill switch"
        );
        match venue_id {
            Some(v) => kill_switch.trip_scope(&risk::Scope::venue(v)),
            None => kill_switch.trip(),
        }
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
    fn a_venue_scoped_divergence_halts_that_venue_and_only_that_venue() {
        // COMP-007 §8.3. The old behaviour halted the whole platform because one
        // venue's bookkeeping disagreed — which is both an over-reaction and the
        // reason an operator reaches for a global reset, losing the information
        // about which venue was actually wrong.
        let ks = ks();
        let result = check_position_divergence_at_venue(
            "BTC-USD",
            "kraken",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("2.0").unwrap(),
            &ks,
        );
        assert!(matches!(result, ReconcileOutcome::Diverged { .. }));
        assert!(ks.refuses(Some("kraken"), None), "the divergent venue halts");
        assert!(
            !ks.refuses(Some("alpaca"), None),
            "the other venues keep trading"
        );
        assert!(!ks.is_active(), "and the platform is not globally halted");
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

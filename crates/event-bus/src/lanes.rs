//! Subject naming helpers for NATS.
//!
//! All NATS subjects follow the pattern `"{lane}.{instrument_id}"`.
//! The quarantine lane is a special single-subject lane with no instrument suffix.

/// Build a NATS subject for a lane + instrument pair.
///
/// # Example
/// ```
/// assert_eq!(event_bus::subject_for("market.trades", "BTC-USD"), "market.trades.BTC-USD");
/// ```
pub fn subject_for(lane: &str, instrument_id: &str) -> String {
    format!("{lane}.{instrument_id}")
}

/// The fixed NATS subject for the quarantine lane.
pub fn quarantine_subject() -> &'static str {
    "quarantine"
}

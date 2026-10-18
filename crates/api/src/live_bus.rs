//! The live frame bus.
//!
//! Panels subscribe over `/ws/live` and the gateway records what each one wants,
//! but until now nothing carried data the other way: the socket acknowledged a
//! subscription and then sent only heartbeats. Every streaming surface in the
//! interface — watchlist quotes, the chart's live tail, the order book, the tape
//! — was therefore permanently empty.
//!
//! This is the missing half. Producers inside the platform publish a
//! [`LiveFrame`]; every open socket receives it and forwards the ones matching
//! its own subscriptions.
//!
//! A broadcast channel is the right shape here: frames are ephemeral, every
//! socket wants its own copy, and a slow consumer must be allowed to miss
//! frames rather than stall the producer. Market data is only useful fresh —
//! `RecvError::Lagged` is handled by skipping ahead, never by replaying.

use std::sync::Arc;

use serde_json::value::RawValue;
use tokio::sync::broadcast;

/// One frame of live data destined for any panel subscribed to `lane` +
/// `instrument`.
///
/// The payload is serialised **once**, at publish time, and the whole frame is
/// shared by `Arc` across every socket (Set C #14/#15). A socket then splices
/// the already-serialised payload text into its own envelope; it never walks
/// or clones a `serde_json::Value` tree per subscriber.
#[derive(Debug)]
pub struct LiveFrame {
    /// The lane as panels name it, e.g. `market.bars.1m`, `ui.orderbook.snapshot`.
    pub lane: String,
    pub instrument: String,
    /// The payload, already serialised.
    pub payload: Box<RawValue>,
}

/// Sender half, held in `AppState` and cloned into every producer.
///
/// The channel carries `Arc<LiveFrame>`, so fan-out to N sockets is N
/// reference-count bumps, not N deep copies.
pub type LiveSender = broadcast::Sender<Arc<LiveFrame>>;

/// Capacity is deliberately generous: a burst on a fast market must not make a
/// slow browser drop frames it could still have rendered within its own
/// repaint budget.
pub const LIVE_BUS_CAPACITY: usize = 4096;

/// Create the bus. The platform binary owns the sender; sockets subscribe.
#[must_use]
pub fn channel() -> LiveSender {
    broadcast::channel(LIVE_BUS_CAPACITY).0
}

/// True when at least one socket is subscribed to the bus.
///
/// The hot path checks this before it formats a frame at all: a market with no
/// open panel is the normal case and must cost nothing.
#[must_use]
pub fn has_listeners(tx: &LiveSender) -> bool {
    tx.receiver_count() > 0
}

/// Publish a frame, ignoring the "no subscribers" case.
///
/// A producer must never care whether anyone is listening — a market with no
/// open panel is the normal case, not an error. This is the cold-path entry:
/// the `Value` is serialised once here.
pub fn publish(tx: &LiveSender, lane: &str, instrument: &str, payload: serde_json::Value) {
    if !has_listeners(tx) {
        return;
    }
    match serde_json::value::to_raw_value(&payload) {
        Ok(raw) => send(tx, lane, instrument, raw),
        Err(e) => tracing::error!(error = %e, lane, "live frame payload failed to serialise"),
    }
}

/// Publish a payload the caller has already rendered as JSON text.
///
/// The hot-path entry: no `serde_json::Value` is ever built. The text is
/// validated once by `RawValue::from_string`; invalid text is dropped and
/// logged rather than forwarded to a browser.
pub fn publish_text(tx: &LiveSender, lane: &str, instrument: &str, json: String) {
    match RawValue::from_string(json) {
        Ok(raw) => send(tx, lane, instrument, raw),
        Err(e) => tracing::error!(error = %e, lane, "live frame payload is not valid JSON"),
    }
}

fn send(tx: &LiveSender, lane: &str, instrument: &str, payload: Box<RawValue>) {
    let _ = tx.send(Arc::new(LiveFrame {
        lane: lane.to_owned(),
        instrument: instrument.to_owned(),
        payload,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_serialisation_is_shared_by_every_receiver() {
        let tx = channel();
        let mut a = tx.subscribe();
        let mut b = tx.subscribe();
        publish(
            &tx,
            "market.trades",
            "BTC-USD",
            serde_json::json!({"price": "1"}),
        );
        let fa = a.try_recv().unwrap();
        let fb = b.try_recv().unwrap();
        // The very same allocation, not an equal copy.
        assert!(Arc::ptr_eq(&fa, &fb));
        assert_eq!(fa.payload.get(), r#"{"price":"1"}"#);
    }

    #[test]
    fn no_listener_means_no_work() {
        let tx = channel();
        assert!(!has_listeners(&tx));
        publish(&tx, "l", "i", serde_json::json!({}));
        let _rx = tx.subscribe();
        assert!(has_listeners(&tx));
    }

    #[test]
    fn invalid_text_is_dropped_not_forwarded() {
        let tx = channel();
        let mut rx = tx.subscribe();
        publish_text(&tx, "l", "i", "{not json".to_owned());
        assert!(rx.try_recv().is_err());
        publish_text(&tx, "l", "i", "{\"a\":1}".to_owned());
        assert!(rx.try_recv().is_ok());
    }
}

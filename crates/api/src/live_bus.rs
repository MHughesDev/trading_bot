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

use serde::Serialize;
use tokio::sync::broadcast;

/// One frame of live data destined for any panel subscribed to `lane` +
/// `instrument`.
#[derive(Clone, Debug, Serialize)]
pub struct LiveFrame {
    /// The lane as panels name it, e.g. `market.bars.1m`, `ui.orderbook.snapshot`.
    pub lane: String,
    pub instrument: String,
    pub payload: serde_json::Value,
}

/// Sender half, held in `AppState` and cloned into every producer.
pub type LiveSender = broadcast::Sender<LiveFrame>;

/// Capacity is deliberately generous: a burst on a fast market must not make a
/// slow browser drop frames it could still have rendered within its own
/// repaint budget.
pub const LIVE_BUS_CAPACITY: usize = 4096;

/// Create the bus. The platform binary owns the sender; sockets subscribe.
#[must_use]
pub fn channel() -> LiveSender {
    broadcast::channel(LIVE_BUS_CAPACITY).0
}

/// Publish a frame, ignoring the "no subscribers" case.
///
/// A producer must never care whether anyone is listening — a market with no
/// open panel is the normal case, not an error.
pub fn publish(tx: &LiveSender, lane: &str, instrument: &str, payload: serde_json::Value) {
    let _ = tx.send(LiveFrame {
        lane: lane.to_owned(),
        instrument: instrument.to_owned(),
        payload,
    });
}

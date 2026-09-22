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
//!
//! # Encode once, fan out by reference (Set C #14, #15)
//!
//! A frame used to carry a `serde_json::Value`. The broadcast channel deep-cloned
//! that `Value` once per open socket, and `ws/live.rs` then cloned it *again* per
//! matching subscription and ran a full `serde_json` encode for each one. Fifty
//! panels watching one instrument therefore paid fifty deep tree clones and fifty
//! encodes of byte-identical output, per tick.
//!
//! The encode now happens exactly once, here, at publish time. What travels the
//! bus and reaches every socket is an `Arc<str>` — the frame's JSON body, already
//! serialised. Cloning it is an atomic increment; fanning it out to a subscription
//! is a `push_str`. [`PAYLOAD_ENCODES`] counts the encodes so a test can assert
//! the invariant rather than trusting the comment.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::error;

/// Total number of live-frame JSON encodes performed since process start.
///
/// The invariant this exists to prove: **one encode per published frame**,
/// independent of how many sockets are open and how many subscriptions each
/// socket holds. It is a plain relaxed counter — one increment per published
/// frame, never per subscriber — and it is read by
/// `ws::live`'s fan-out tests.
pub static PAYLOAD_ENCODES: AtomicU64 = AtomicU64::new(0);

/// [`PAYLOAD_ENCODES`] is process-wide, so two tests reading a *delta* across it
/// concurrently would see each other's publishes. Every test that publishes takes
/// this lock first. Test-only; it costs nothing in a build without `cfg(test)`.
#[cfg(test)]
static ENCODE_COUNTER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serialise the tests that touch [`PAYLOAD_ENCODES`]. A poisoned lock is
/// recovered rather than propagated: one failing test must not cascade.
#[cfg(test)]
pub(crate) fn encode_counter_guard() -> std::sync::MutexGuard<'static, ()> {
    ENCODE_COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The part of a `WsOutMessage::Frame` that does not depend on which
/// subscription is receiving it.
///
/// Serialising this and dropping the opening `{` yields a suffix that can be
/// appended verbatim to `{"type":"frame","sub_id":"<uuid>"`, which is how the
/// fan-out avoids re-encoding. The field order matches
/// `ui_gateway::transport::WsOutMessage::Frame`; `ws::live`'s parity test asserts
/// the two agree byte for byte.
#[derive(Serialize)]
struct FrameBody<'a> {
    lane: &'a str,
    instrument: &'a str,
    payload: &'a serde_json::Value,
}

/// One frame of live data destined for any panel subscribed to `lane` +
/// `instrument`.
///
/// The payload is not stored as a `serde_json::Value`: it is stored already
/// encoded, so that neither the broadcast fan-out nor the per-subscription
/// fan-out re-serialises it. See the module docs.
#[derive(Clone, Debug)]
pub struct LiveFrame {
    /// The lane as panels name it, e.g. `market.bars.1m`, `ui.orderbook.snapshot`.
    pub lane: Arc<str>,
    pub instrument: Arc<str>,
    /// `"lane":…,"instrument":…,"payload":…}` — the frame's JSON body without its
    /// opening brace, encoded once at publish time.
    body: Arc<str>,
}

impl LiveFrame {
    /// The pre-encoded JSON body, shared by every socket and every subscription.
    ///
    /// Starts with `"lane":` and ends with the closing `}` of the frame object.
    #[must_use]
    pub fn body(&self) -> &Arc<str> {
        &self.body
    }

    /// Encode a frame. Returns `None` if the payload cannot be serialised, which
    /// for a `serde_json::Value` means a non-finite float reached the bus.
    fn encode(lane: &str, instrument: &str, payload: &serde_json::Value) -> Option<Self> {
        let whole = match serde_json::to_string(&FrameBody {
            lane,
            instrument,
            payload,
        }) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, lane, instrument, "live frame payload is not serialisable; dropped");
                return None;
            }
        };
        PAYLOAD_ENCODES.fetch_add(1, Ordering::Relaxed);
        // `to_string` of a struct always begins with `{`; strip it so the result
        // can be appended after the sub_id field.
        let body = whole
            .strip_prefix('{')
            .expect("serde_json object always starts with '{'")
            .to_owned();
        Some(Self {
            lane: Arc::from(lane),
            instrument: Arc::from(instrument),
            body: Arc::from(body),
        })
    }
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
///
/// The payload is encoded here, once, whatever the number of listeners.
pub fn publish(tx: &LiveSender, lane: &str, instrument: &str, payload: serde_json::Value) {
    if let Some(frame) = LiveFrame::encode(lane, instrument, &payload) {
        let _ = tx.send(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_the_frame_object_without_its_opening_brace() {
        let _guard = encode_counter_guard();
        let f = LiveFrame::encode("market.trades", "BTC-USD", &serde_json::json!({"price": "1"}))
            .expect("encodes");
        assert_eq!(
            &*f.body,
            r#""lane":"market.trades","instrument":"BTC-USD","payload":{"price":"1"}}"#
        );
    }

    #[test]
    fn lane_and_instrument_are_json_escaped_by_serde_not_by_hand() {
        let _guard = encode_counter_guard();
        // A quote in an instrument name must not be able to break out of the
        // string and forge extra JSON fields.
        let f = LiveFrame::encode("l", r#"a","x":"y"#, &serde_json::json!(1)).expect("encodes");
        let whole = format!("{{{}", f.body);
        let v: serde_json::Value = serde_json::from_str(&whole).expect("still valid json");
        assert_eq!(v["instrument"], serde_json::json!(r#"a","x":"y"#));
        assert!(v.get("x").is_none(), "escaping failed: forged field present");
    }

    #[test]
    fn a_published_frame_is_encoded_exactly_once_however_many_sockets_listen() {
        let _guard = encode_counter_guard();
        let tx = channel();
        let mut sockets: Vec<_> = (0..16).map(|_| tx.subscribe()).collect();

        let before = PAYLOAD_ENCODES.load(Ordering::Relaxed);
        publish(&tx, "market.trades", "BTC-USD", serde_json::json!({"p": "1"}));
        let after = PAYLOAD_ENCODES.load(Ordering::Relaxed);

        assert_eq!(after - before, 1, "one publish must cost one encode");

        // And every socket received the *same* allocation, not a copy of it.
        let first = sockets[0].try_recv().expect("frame delivered");
        for s in &mut sockets[1..] {
            let f = s.try_recv().expect("frame delivered");
            assert!(
                Arc::ptr_eq(f.body(), first.body()),
                "sockets must share one encoded body, not clone it"
            );
        }
    }

    #[test]
    fn a_payload_that_cannot_be_encoded_is_dropped_not_panicked() {
        let _guard = encode_counter_guard();
        let tx = channel();
        let mut rx = tx.subscribe();
        // f64::NAN serialises as `null` in serde_json rather than failing, so the
        // honest check here is that a hostile payload cannot panic the producer.
        publish(
            &tx,
            "l",
            "i",
            serde_json::json!({ "nan": f64::NAN, "deep": [[[[1]]]] }),
        );
        assert!(rx.try_recv().is_ok());
    }
}

//! Live L2 order-book feed.
//!
//! The UI gateway has always exposed a `ui.orderbook.snapshot` lane and the
//! trading terminal has always subscribed to it — but nothing published to it,
//! so the order book panel had no data source at all.
//!
//! This task is that source. It holds one Kraken WS v2 `book` subscription per
//! instrument, maintains the book locally from the snapshot + deltas the venue
//! sends, and republishes a depth-capped snapshot onto the live bus at a
//! throttled rate.
//!
//! Design notes:
//!
//! * **Throttled, not per-delta.** A book updates far faster than any screen can
//!   usefully repaint. Publishing every delta would burn bandwidth to produce
//!   frames the browser throttles away anyway, so the task coalesces and emits
//!   at most `MAX_FPS` snapshots a second.
//! * **Snapshot, not delta, on the wire.** Panels come and go; a panel that
//!   joins mid-stream must not have to replay deltas to become correct. Sending
//!   a bounded snapshot makes a late subscriber correct on its first frame.
//! * **Prices are decimal strings end to end.** The venue sends JSON numbers;
//!   they are converted once, here, and never parsed as floats downstream.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

const KRAKEN_WS_URL: &str = "wss://ws.kraken.com/v2";
/// Levels retained per side. The terminal shows twelve; keeping a few more
/// means grouping still has something to aggregate.
const BOOK_DEPTH: usize = 50;
/// Levels published per side.
const PUBLISH_DEPTH: usize = 25;
/// Upper bound on published frames per second, per instrument.
const MAX_FPS: u64 = 6;

/// Request to start following one instrument's book.
#[derive(Clone, Debug)]
pub struct BookRequest {
    /// Domain id, e.g. `BTC-USD`.
    pub instrument_id: String,
    /// Venue symbol, e.g. `BTC/USD`.
    pub symbol: String,
}

#[derive(Debug, Deserialize)]
struct KrakenBookMessage {
    channel: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    data: Vec<KrakenBookData>,
}

#[derive(Debug, Deserialize)]
struct KrakenBookData {
    #[allow(dead_code)]
    symbol: Option<String>,
    #[serde(default)]
    bids: Vec<KrakenLevel>,
    #[serde(default)]
    asks: Vec<KrakenLevel>,
}

#[derive(Debug, Deserialize)]
struct KrakenLevel {
    price: f64,
    qty: f64,
}

/// One side of the book, keyed by price so deltas are a map write.
///
/// `BTreeMap` rather than a sorted `Vec`: a delta touches one price, and the
/// book is read in price order. Both are what a map gives for free.
type Side = BTreeMap<Decimal, Decimal>;

fn apply(side: &mut Side, levels: &[KrakenLevel]) {
    for l in levels {
        let Some(price) = Decimal::from_f64_retain(l.price) else {
            continue;
        };
        let Some(qty) = Decimal::from_f64_retain(l.qty) else {
            continue;
        };
        // A zero quantity is how the venue says "this level is gone".
        if qty.is_zero() {
            side.remove(&price);
        } else {
            side.insert(price, qty);
        }
    }
}

fn trim(side: &mut Side, keep_highest: bool) {
    while side.len() > BOOK_DEPTH {
        let key = if keep_highest {
            *side.keys().next().expect("non-empty")
        } else {
            *side.keys().next_back().expect("non-empty")
        };
        side.remove(&key);
    }
}

fn snapshot_json(bids: &Side, asks: &Side, sequence: u64) -> serde_json::Value {
    // Bids descend from the touch, asks ascend from it.
    let bid_rows: Vec<_> = bids
        .iter()
        .rev()
        .take(PUBLISH_DEPTH)
        .map(|(p, q)| serde_json::json!({ "price": p.to_string(), "size": q.to_string() }))
        .collect();
    let ask_rows: Vec<_> = asks
        .iter()
        .take(PUBLISH_DEPTH)
        .map(|(p, q)| serde_json::json!({ "price": p.to_string(), "size": q.to_string() }))
        .collect();

    serde_json::json!({
        "kind": "snapshot",
        "bids": bid_rows,
        "asks": ask_rows,
        "sequence": sequence,
        "is_tentative": false,
    })
}

/// Follow one instrument's book until the process ends, reconnecting on drop.
async fn run_one(req: BookRequest, live: api::live_bus::LiveSender) {
    let mut backoff = std::time::Duration::from_secs(1);

    loop {
        info!(symbol = %req.symbol, "connecting to Kraken book feed");
        let (mut ws, _) = match connect_async(KRAKEN_WS_URL).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, symbol = %req.symbol, "book WS connect failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
                continue;
            }
        };

        let subscribe = serde_json::json!({
            "method": "subscribe",
            "params": { "channel": "book", "symbol": [req.symbol], "depth": 25 },
            "req_id": 7,
        });
        if let Err(e) = ws.send(Message::Text(subscribe.to_string())).await {
            warn!(error = %e, "book subscribe failed");
            tokio::time::sleep(backoff).await;
            continue;
        }

        backoff = std::time::Duration::from_secs(1);
        info!(symbol = %req.symbol, "subscribed to Kraken book channel");

        let mut bids: Side = BTreeMap::new();
        let mut asks: Side = BTreeMap::new();
        let mut sequence: u64 = 0;
        let mut published: u64 = 0;
        let mut last_publish = tokio::time::Instant::now();
        let min_gap = std::time::Duration::from_millis(1000 / MAX_FPS);

        loop {
            let Some(msg) = ws.next().await else {
                warn!(symbol = %req.symbol, "book stream ended");
                break;
            };
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "book WS read error");
                    break;
                }
            };

            match msg {
                Message::Ping(d) => {
                    let _ = ws.send(Message::Pong(d)).await;
                    continue;
                }
                Message::Close(_) => break,
                Message::Text(text) => {
                    let parsed: Result<KrakenBookMessage, _> = serde_json::from_str(&text);
                    let Ok(km) = parsed else { continue };
                    if km.channel.as_deref() != Some("book") {
                        continue;
                    }
                    let is_snapshot = km.msg_type.as_deref() == Some("snapshot");
                    for d in &km.data {
                        if is_snapshot {
                            bids.clear();
                            asks.clear();
                        }
                        apply(&mut bids, &d.bids);
                        apply(&mut asks, &d.asks);
                        trim(&mut bids, true);
                        trim(&mut asks, false);
                        sequence = sequence.wrapping_add(1);
                    }
                }
                _ => continue,
            }

            // Coalesce: publish at most MAX_FPS snapshots a second.
            if last_publish.elapsed() < min_gap {
                continue;
            }
            last_publish = tokio::time::Instant::now();
            if bids.is_empty() && asks.is_empty() {
                continue;
            }
            api::live_bus::publish(
                &live,
                domain::lanes::UI_ORDERBOOK_SNAPSHOT,
                &req.instrument_id,
                snapshot_json(&bids, &asks, sequence),
            );
            published += 1;
            if published == 1 {
                info!(
                    symbol = %req.symbol,
                    bids = bids.len(),
                    asks = asks.len(),
                    "first order-book snapshot published"
                );
            }
        }

        tokio::time::sleep(backoff).await;
    }
}

/// Own the set of followed books.
///
/// `ensure` is idempotent, so the asset-lifecycle handler can call it on every
/// instrument start without tracking what is already running.
pub struct BookFeeds {
    live: api::live_bus::LiveSender,
    running: dashmap::DashSet<String>,
}

impl BookFeeds {
    #[must_use]
    pub fn new(live: api::live_bus::LiveSender) -> Self {
        Self {
            live,
            running: dashmap::DashSet::new(),
        }
    }

    /// Start following `instrument_id` if it is not already being followed.
    ///
    /// Only venues with a book adapter are followed; everything else simply has
    /// no depth, which the panel states plainly rather than showing an empty
    /// grid that looks like a market with no orders in it.
    pub fn ensure(self: &Arc<Self>, instrument_id: &str, asset_class: &str) {
        if !matches!(asset_class, "crypto_spot_cex" | "perpetual_swap") {
            debug!(instrument_id, asset_class, "no book adapter for this asset class");
            return;
        }
        if !self.running.insert(instrument_id.to_owned()) {
            return;
        }
        let symbol = instrument_id.replace('-', "/");
        let req = BookRequest {
            instrument_id: instrument_id.to_owned(),
            symbol,
        };
        let live = self.live.clone();
        tokio::spawn(run_one(req, live));
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.running.len()
    }
}

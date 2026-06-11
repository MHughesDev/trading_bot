//! JetStream tee task — receives RawTick events from the hot-path socket reader
//! via an unbounded mpsc channel and publishes them to JetStream asynchronously.
//!
//! This task is the ONLY place in the platform that calls `Publisher::publish`.
//! The strategy evaluation path never touches this task.

use std::sync::Arc;

use tracing::{info, warn};

use crate::hot_path::RawTick;

/// Drain `tee_rx` and publish each tick to JetStream without awaiting ACK.
///
/// If this task falls behind the socket reader the channel grows unboundedly
/// — acceptable because JetStream writes are best-effort for replay, not for
/// live decisions.  A future improvement (set-C issue #35) can bound this.
pub async fn run_tee(
    publisher: Arc<event_bus::Publisher>,
    mut tee_rx: tokio::sync::mpsc::UnboundedReceiver<RawTick>,
) {
    info!("tee task starting");
    while let Some(tick) = tee_rx.recv().await {
        let instrument_name = domain::instrument_name(tick.instrument_id)
            .unwrap_or_else(|| String::from("unknown"));
        publisher.publish_fire_and_forget(&tick, &instrument_name, domain::lanes::MARKET_TRADES);
    }
    warn!("tee task channel closed — no more events will be persisted to JetStream");
}

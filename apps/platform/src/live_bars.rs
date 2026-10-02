//! Completed 1-minute bars, process-wide (Set U U-4.1).
//!
//! Each pipeline's stage 2 already builds the minute bar it persists to
//! ClickHouse. It now also publishes it here, and every v3 live runner
//! subscribes: the engine computes its own indicators from these bars, so a
//! strategy needs no feature service and no per-tick snapshot to decide.
//!
//! A broadcast channel, so a slow runner cannot hold up the pipeline: a runner
//! that falls behind by more than the buffer is told it lagged, and says so.

use std::sync::OnceLock;

use tokio::sync::broadcast;

/// One completed minute of one instrument.
#[derive(Clone, Debug)]
pub struct MinuteBar {
    pub instrument_id: String,
    pub bar: backtest::CollectedBar,
}

const CAPACITY: usize = 8_192;

fn hub() -> &'static broadcast::Sender<MinuteBar> {
    static HUB: OnceLock<broadcast::Sender<MinuteBar>> = OnceLock::new();
    HUB.get_or_init(|| broadcast::channel(CAPACITY).0)
}

/// Publish a completed minute. Never blocks; with no subscriber it is a no-op.
pub fn publish(instrument_id: &str, bar: &backtest::CollectedBar) {
    let _ = hub().send(MinuteBar {
        instrument_id: instrument_id.to_owned(),
        bar: bar.clone(),
    });
}

/// Every completed minute published from now on.
#[must_use]
pub fn subscribe() -> broadcast::Receiver<MinuteBar> {
    hub().subscribe()
}

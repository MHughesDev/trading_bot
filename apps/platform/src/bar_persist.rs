//! Continuous 1-minute bar persistence task.
//!
//! Every initialized asset's pipeline aggregates live ticks into 1-minute OHLCV
//! bars (see `hot_path::stage_bar_builder`) and sends each completed bar here.
//! This task is the single writer of live bars to the ClickHouse `market_bars`
//! table, so an initialized asset keeps accumulating minute-level history for as
//! long as the platform runs — independent of whether any strategy or
//! automation is subscribed to it.
//!
//! Persistence is best-effort and off the hot path: a slow or unavailable
//! ClickHouse never stalls the socket reader or the mark board.

use backtest::{BarStore, CollectedBar};
use domain::payloads::bar::Timeframe;
use tracing::{debug, info, warn};

/// A completed 1-minute bar plus the routing metadata needed to write it.
pub struct PersistBar {
    pub instrument_id: String,
    pub venue_id: String,
    pub source: String,
    pub trust_tier: String,
    pub bar: CollectedBar,
}

/// Drain `rx` and write each completed 1-minute bar to ClickHouse.
///
/// Bars arrive at most once per minute per instrument, so a per-bar insert is
/// cheap; batching is unnecessary.  A failed insert is logged and dropped — the
/// next minute's bar is independent, and historical gaps can be backfilled by
/// the collector path.
pub async fn run_bar_persist(
    clickhouse_url: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<PersistBar>,
) {
    info!("bar-persist task starting (live 1m → ClickHouse market_bars)");
    let store = BarStore::connect(&clickhouse_url);

    while let Some(item) = rx.recv().await {
        match store
            .insert_collected(
                &item.instrument_id,
                &item.venue_id,
                &item.source,
                &item.trust_tier,
                Timeframe::Minutes1,
                std::slice::from_ref(&item.bar),
            )
            .await
        {
            Ok(()) => debug!(
                instrument_id = %item.instrument_id,
                close = %item.bar.close,
                "live 1m bar persisted"
            ),
            Err(e) => warn!(
                instrument_id = %item.instrument_id,
                error = %e,
                "failed to persist live 1m bar"
            ),
        }
    }
    warn!("bar-persist channel closed — live 1m bars will no longer be stored");
}

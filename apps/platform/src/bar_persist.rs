//! Continuous 1-minute bar persistence task.
//!
//! Every initialized asset's pipeline aggregates live ticks into 1-minute OHLCV
//! bars (see `hot_path::stage_bar_builder`) and sends each completed bar here.
//! This task is the single writer of live bars to the canonical ClickHouse `market_bar`
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
    live: api::live_bus::LiveSender,
) {
    info!("bar-persist task starting (live 1m → ClickHouse market_bar + live bus)");
    let store = BarStore::connect(&clickhouse_url);

    while let Some(item) = rx.recv().await {
        // Fan the bar out to any open panel first. Persistence is best-effort
        // and can be slow; a chart waiting on ClickHouse to draw its live tail
        // would be a chart that lags the market for no reason.
        api::live_bus::publish(
            &live,
            domain::lanes::MARKET_BARS_1M,
            &item.instrument_id,
            serde_json::json!({
                // Unix seconds: what lightweight-charts wants on the x axis.
                "ts": item.bar.available_time.timestamp(),
                "open": item.bar.open,
                "high": item.bar.high,
                "low": item.bar.low,
                "close": item.bar.close,
                "volume": item.bar.volume,
                "trade_count": item.bar.trade_count,
                "venue_id": item.venue_id,
                "source": item.source,
            }),
        );

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

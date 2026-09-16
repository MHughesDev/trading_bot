//! `BarStore` as the backfill's only writer (Set P P-1.2…P-1.5; DA-16, DA-17;
//! INV-02).
//!
//! The backfill engine holds `dataplane::backfill::BarSink`, not a database
//! handle. This is the one implementation that reaches storage, and it is
//! deliberately thin — its whole job is to hand the adapter's bars, and the
//! adapter's **source name**, to `BarStore::write_collected`, which then:
//!
//! * resolves `(venue, symbol)` to a surrogate instrument id;
//! * reads `declared_vendor_lag_ms` for that source from `dataplane.source` and
//!   stamps `knowledge_time = bar close + lag` with
//!   `BACKFILLED_KNOWLEDGE_TIME` (ADR-P2-24);
//! * writes a **new revision row** when a cell's values changed, never an
//!   update (DA-16);
//! * writes to `market_bar`, never the frozen `market_bars`
//!   (`cargo xtask check-bars-v1-frozen`).
//!
//! Nothing in that list is re-implemented here, and that is the point: a second
//! writer is a second place for the knowledge-time convention to be wrong, and
//! the last time REST backfills were stamped with their close time the whole
//! platform's history looked observed-live.
//!
//! **One timeframe per call.** DA-17: writing several timeframes of one
//! instrument in a single batch destroyed the coarse bars instantly under the v1
//! schema. v2 fixed the schema, but the shape of this interface keeps the
//! batches separate regardless.

use std::sync::Arc;

use async_trait::async_trait;
use dataplane::backfill::{BackfillBar, BarSink, SinkError, SinkReport};
use domain::payloads::bar::Timeframe;

use crate::store::{BarStore, CollectedBar};
use crate::types::TimeframeExt;

/// Writes backfilled bars through the single PIT store.
pub struct BarStoreSink {
    store: Arc<BarStore>,
}

impl BarStoreSink {
    #[must_use]
    pub const fn new(store: Arc<BarStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl BarSink for BarStoreSink {
    async fn write_page(
        &self,
        source: &str,
        venue: &str,
        instrument: &str,
        timeframe: &str,
        bars: &[BackfillBar],
    ) -> Result<SinkReport, SinkError> {
        let tf: Timeframe = <Timeframe as TimeframeExt>::from_key(timeframe)
            .ok_or_else(|| SinkError::Backend(format!("unknown timeframe {timeframe:?}")))?;
        let collected: Vec<CollectedBar> = bars
            .iter()
            .map(|b| CollectedBar {
                // `CollectedBar::available_time` is the bar CLOSE, which is what
                // `BackfillBar::close_time` already holds. The conversion from
                // whatever the venue publishes happened once, in the adapter.
                available_time: b.close_time,
                sequence: u64::try_from(b.close_time.timestamp()).unwrap_or(0),
                open: b.open.clone(),
                high: b.high.clone(),
                low: b.low.clone(),
                close: b.close.clone(),
                volume: b.volume.clone(),
                trade_count: b.trade_count,
            })
            .collect();

        let r = self
            .store
            // `source` is passed through from the adapter, never defaulted: it
            // is what selects the declared vendor lag, and defaulting it would
            // give Alpaca's 900 s-delayed bars Coinbase's 60 s knowledge time.
            .write_collected(instrument, venue, source, tf, &collected)
            .await
            .map_err(|e| SinkError::Backend(e.to_string()))?;

        Ok(SinkReport {
            inserted: r.inserted as u64,
            restated: r.restated as u64,
            unchanged: r.unchanged as u64,
            rejected: r.rejected as u64,
        })
    }
}

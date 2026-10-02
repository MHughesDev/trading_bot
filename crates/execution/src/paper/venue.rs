//! [`PaperVenue`] — the paper driver behind the [`Venue`] port (Set U U-2.5).
//!
//! A thin view over the shared [`PaperTradingEngine`]: the order path is the
//! engine's own `submit`, the one every paper hot-path test exercises. Marks
//! arrive from the live market-data feed through [`PaperTradingEngine::on_mark`]
//! as they always have; this type adds nothing to the fill path.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::{instrument::AssetClass, order::OrderIntent};
use uuid::Uuid;

use super::{PaperAccountSnapshot, PaperTradeError, PaperTradingEngine};
use crate::broker::{BrokerOrderStatus, BrokerPosition};
use crate::venue_port::Venue;

/// The paper venue: wall clock, live marks, the shared engine.
#[derive(Clone)]
pub struct PaperVenue {
    engine: Arc<PaperTradingEngine>,
}

impl PaperVenue {
    #[must_use]
    pub const fn new(engine: Arc<PaperTradingEngine>) -> Self {
        Self { engine }
    }

    /// The engine behind it.
    #[must_use]
    pub fn engine(&self) -> &Arc<PaperTradingEngine> {
        &self.engine
    }
}

impl Venue for PaperVenue {
    fn submit(
        &self,
        asset_class: AssetClass,
        intent: &OrderIntent,
    ) -> Result<Uuid, PaperTradeError> {
        self.engine.submit(asset_class, intent)?;
        Ok(intent.idempotency_key)
    }

    fn cancel(&self, key: Uuid) -> Result<(), PaperTradeError> {
        let id = self
            .engine
            .order_id_for(key)
            .ok_or_else(|| PaperTradeError::UnknownOrder(key.to_string()))?;
        self.engine.cancel(&id)
    }

    fn order(&self, key: Uuid) -> Option<BrokerOrderStatus> {
        self.engine
            .order_id_for(key)
            .and_then(|id| self.engine.order_status(&id))
    }

    fn open_orders(&self, asset_class: Option<AssetClass>) -> Vec<BrokerOrderStatus> {
        self.engine.open_orders(asset_class)
    }

    fn positions(&self, asset_class: AssetClass) -> Vec<BrokerPosition> {
        self.engine.positions(asset_class)
    }

    fn snapshot(&self, asset_class: AssetClass) -> PaperAccountSnapshot {
        self.engine.snapshot(asset_class)
    }

    fn now(&self) -> DateTime<Utc> {
        self.engine.now()
    }
}

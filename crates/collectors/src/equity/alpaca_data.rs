//! Alpaca data feed collector stub.
//!
//! Full implementation is deferred to Phase 6 (equity hours / halt-aware).

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use crate::{Collector, CollectorError};

/// Alpaca equity data feed connector.
pub struct AlpacaDataCollector {
    pub symbol: String,
}

impl AlpacaDataCollector {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
        }
    }
}

#[async_trait]
impl Collector for AlpacaDataCollector {
    async fn run(
        &self,
        _publisher: Arc<event_bus::Publisher>,
        _quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        warn!(symbol = %self.symbol, "AlpacaDataCollector not yet implemented");
        Err(CollectorError::Fatal("not implemented".into()))
    }
}

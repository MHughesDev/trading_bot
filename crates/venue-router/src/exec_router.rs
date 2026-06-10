//! Execution router — dispatches orders to paper simulators or live venue adapters.
//!
//! The router has exactly two modes: `Paper` (internal simulator, no network) and
//! `LiveRouted` (venue broker adapter). Internal validity checks may reject malformed
//! orders but are NOT labeled "risk" (C-058/C-059).

use std::collections::HashMap;
use std::sync::Arc;

use domain::{instrument::AssetClass, money::Price, order::OrderIntent};
use execution::{
    broker::Broker,
    paper::{simulator_for, PaperFill},
};
use thiserror::Error;

/// Which execution path to take for this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Paper — fill via internal simulator; no venue call made.
    Paper,
    /// Live — submit to the venue broker adapter.
    LiveRouted,
}

/// Outcome of routing an order.
#[derive(Debug)]
pub enum RouteOutcome {
    /// Paper path: immediate simulated fill returned.
    PaperFill(PaperFill),
    /// Live path: broker order ID returned; fill arrives asynchronously.
    LiveSubmitted { broker_order_id: String },
}

/// Errors that the router raises.  Not called "risk" — these are routing/validity issues.
#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("no live adapter registered for venue: {0}")]
    NoAdapter(String),
    #[error("unsupported order type for venue {venue}: {detail}")]
    UnsupportedOrderType { venue: String, detail: String },
    #[error("broker error: {0}")]
    Broker(#[from] execution::broker::BrokerError),
}

/// Routes orders to the correct execution path.
pub struct ExecRouter {
    /// Live broker adapters keyed by venue slug.
    adapters: HashMap<String, Arc<dyn Broker>>,
}

impl ExecRouter {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register a live broker adapter for a venue.
    pub fn register(&mut self, venue: &str, adapter: Arc<dyn Broker>) {
        self.adapters.insert(venue.to_owned(), adapter);
    }

    /// Route an order.
    ///
    /// - `Paper`: resolves the market structure from `asset_class`, calls the
    ///   matching simulator, returns an immediate `PaperFill`.  No network call.
    /// - `LiveRouted`: looks up the adapter by `venue`, submits the approved order,
    ///   returns the broker order ID.
    pub async fn route(
        &self,
        mode: ExecutionMode,
        asset_class: AssetClass,
        venue: &str,
        intent: &OrderIntent,
        mark: Price,
        approved: &risk::ApprovedOrder,
    ) -> Result<RouteOutcome, RoutingError> {
        match mode {
            ExecutionMode::Paper => {
                let structure = asset_class.market_structure();
                let simulator = simulator_for(structure);
                let fill = simulator.simulate_fill(intent, mark);
                Ok(RouteOutcome::PaperFill(fill))
            }
            ExecutionMode::LiveRouted => {
                let adapter = self
                    .adapters
                    .get(venue)
                    .ok_or_else(|| RoutingError::NoAdapter(venue.to_owned()))?;

                let broker_order_id = adapter.submit(approved).await?;
                Ok(RouteOutcome::LiveSubmitted { broker_order_id })
            }
        }
    }
}

impl Default for ExecRouter {
    fn default() -> Self {
        Self::new()
    }
}

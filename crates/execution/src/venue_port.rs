//! The one port every in-process venue sits behind (Set U U-2.5, ADR-0044).
//!
//! Backtest and paper "work similarly but not exactly the same": the same venue
//! core ([`crate::paper::PaperTradingEngine`]) books every order, position and
//! fee in both, and the two differ only in where prices and time come from. An
//! environment holds a `dyn Venue` and never learns which one it has.
//!
//! Orders are addressed by the intent's **idempotency key**, not the venue's
//! order id, because in a backtest an order submitted at a bar's close does not
//! reach the venue until the next bar's open: between the two it has a key and
//! no id. The key is also what the strategy engine's planner already holds.
//!
//! The live brokers are not behind this port. They are asynchronous, sit behind
//! the durable order journal, and answer to [`crate::broker::Broker`]; nothing
//! here is meant to reach them.

use chrono::{DateTime, Utc};
use domain::{instrument::AssetClass, order::OrderIntent};
use uuid::Uuid;

use crate::broker::{BrokerOrderStatus, BrokerPosition};
use crate::paper::{PaperAccountSnapshot, PaperTradeError};

/// An in-process venue: paper (wall clock, live marks) or simulated (bars, a
/// simulated clock).
pub trait Venue: Send + Sync {
    /// Hand the venue an order. Returns its idempotency key. Resubmitting the
    /// same key is a no-op that returns the key again.
    ///
    /// # Errors
    /// Anything the venue refuses *now*. An order the simulated venue holds for
    /// the next bar is refused (or not) when it arrives, and the refusal is in
    /// that bar's report.
    fn submit(
        &self,
        asset_class: AssetClass,
        intent: &OrderIntent,
    ) -> Result<Uuid, PaperTradeError>;

    /// Cancel by key. Cancelling a terminal order is a no-op.
    ///
    /// # Errors
    /// An unknown key.
    fn cancel(&self, key: Uuid) -> Result<(), PaperTradeError>;

    /// The order's status, once it has reached the venue.
    fn order(&self, key: Uuid) -> Option<BrokerOrderStatus>;

    /// Open orders, optionally for one asset class.
    fn open_orders(&self, asset_class: Option<AssetClass>) -> Vec<BrokerOrderStatus>;

    /// Positions in one asset class's account.
    fn positions(&self, asset_class: AssetClass) -> Vec<BrokerPosition>;

    /// One asset class's account.
    fn snapshot(&self, asset_class: AssetClass) -> PaperAccountSnapshot;

    /// The venue's time: the wall for paper, the simulated clock in a backtest.
    fn now(&self) -> DateTime<Utc>;
}

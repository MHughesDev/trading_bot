//! `PaperTradingEngine` — the internal execution venue for all asset classes.
//!
//! One engine instance owns:
//! - a [`PaperAccount`] per asset class (opened and seeded at startup),
//! - a per-instrument mark-price board fed by the hot path via [`PaperTradingEngine::on_mark`],
//! - an internal order store with resting limit orders that fill when the
//!   mark crosses their price.
//!
//! **No external calls anywhere** — every fill, balance, position, and
//! transaction is computed and stored in this process.
//!
//! Lock ordering: an `orders` map entry may be held while taking an account
//! mutex (resting-order fills do this); the reverse never happens.  Account
//! mutexes are always released before touching the order/resting maps.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use domain::{
    instrument::{AssetClass, MarketStructure},
    money::{Price, Size},
    order::{OrderIntent, OrderType, TimeInForce},
};
use rust_decimal::Decimal;
use tracing::{debug, info};
use uuid::Uuid;

use crate::broker::{BrokerOrderState, BrokerOrderStatus, BrokerPosition};

use super::account::{PaperAccount, PaperAccountSnapshot, PaperTradeError};
use super::ledger::PaperLedgerEntry;
use super::policy::{AccountPolicy, ALL_ASSET_CLASSES};
use super::{
    AmmQuoteSwapSimulator, BrokerQuoteFillSimulator, ClobFillSimulator, PaperFill,
    PaperFillSimulator, PredictionMarketFillSimulator,
};

/// Terminal orders retained for `query_order` before FIFO eviction.
const ORDER_HISTORY_CAPACITY: usize = 50_000;

/// The four market-structure simulators, instantiated once per engine.
#[derive(Debug, Default)]
pub struct SimulatorSet {
    pub clob: ClobFillSimulator,
    pub broker_quote: BrokerQuoteFillSimulator,
    pub amm: AmmQuoteSwapSimulator,
    pub prediction: PredictionMarketFillSimulator,
}

impl SimulatorSet {
    fn for_structure(&self, structure: MarketStructure) -> &dyn PaperFillSimulator {
        match structure {
            MarketStructure::Clob => &self.clob,
            MarketStructure::BrokerQuote => &self.broker_quote,
            MarketStructure::AmmSwap => &self.amm,
            MarketStructure::PredictionBinary => &self.prediction,
        }
    }
}

/// Internal record of one paper order (live or terminal).
#[derive(Clone, Debug)]
pub struct PaperOrderRecord {
    pub order_id: String,
    pub asset_class: AssetClass,
    pub intent: OrderIntent,
    pub filled_qty: Decimal,
    pub avg_fill_price: Option<Price>,
    pub state: BrokerOrderState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PaperOrderRecord {
    fn new(order_id: String, asset_class: AssetClass, intent: OrderIntent) -> Self {
        let now = Utc::now();
        Self {
            order_id,
            asset_class,
            intent,
            filled_qty: Decimal::ZERO,
            avg_fill_price: None,
            state: BrokerOrderState::New,
            created_at: now,
            updated_at: now,
        }
    }

    /// Fold a fill into the record's cumulative quantity / VWAP / state.
    fn record_fill(&mut self, fill: &PaperFill) {
        let prev_notional = self
            .avg_fill_price
            .map_or(Decimal::ZERO, |p| p.inner() * self.filled_qty);
        self.filled_qty += fill.filled_qty;
        if self.filled_qty > Decimal::ZERO {
            let vwap =
                (prev_notional + fill.fill_price.inner() * fill.filled_qty) / self.filled_qty;
            self.avg_fill_price = Some(Price::from_decimal(vwap));
        }
        self.state = if self.filled_qty >= self.intent.size.inner() {
            BrokerOrderState::Filled
        } else {
            BrokerOrderState::PartiallyFilled
        };
        self.updated_at = Utc::now();
    }

    fn is_open(&self) -> bool {
        matches!(
            self.state,
            BrokerOrderState::New | BrokerOrderState::PartiallyFilled
        )
    }

    fn to_status(&self) -> BrokerOrderStatus {
        BrokerOrderStatus {
            broker_order_id: self.order_id.clone(),
            instrument_id: self.intent.instrument_id.clone(),
            side: self.intent.side,
            order_type: self.intent.order_type,
            submitted_qty: self.intent.size.inner(),
            filled_qty: self.filled_qty,
            avg_fill_price: self.avg_fill_price,
            state: self.state.clone(),
        }
    }
}

/// Internal paper execution venue covering every asset class.
pub struct PaperTradingEngine {
    accounts: HashMap<AssetClass, Mutex<PaperAccount>>,
    /// Latest mark per instrument id, written by the hot path.
    marks: DashMap<String, Price>,
    /// Every order the engine has seen, keyed by engine order id.
    orders: DashMap<String, PaperOrderRecord>,
    /// Resting (open limit) order ids per instrument id.
    resting: DashMap<String, Vec<String>>,
    /// Submitted idempotency keys → engine order id (resubmit is a no-op).
    idempotency: DashMap<Uuid, String>,
    /// FIFO of terminal order ids for bounded history eviction.
    terminal_fifo: Mutex<VecDeque<String>>,
    sims: SimulatorSet,
}

impl Default for PaperTradingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PaperTradingEngine {
    /// Engine with default policies and simulators; every asset class gets an
    /// account seeded with its policy's default starting cash.
    pub fn new() -> Self {
        Self::with_config(&HashMap::new(), SimulatorSet::default())
    }

    /// Engine with per-class starting-cash overrides and custom simulators.
    pub fn with_config(
        starting_cash_overrides: &HashMap<AssetClass, Decimal>,
        sims: SimulatorSet,
    ) -> Self {
        let accounts = ALL_ASSET_CLASSES
            .into_iter()
            .map(|ac| {
                let cash = starting_cash_overrides
                    .get(&ac)
                    .copied()
                    .unwrap_or_else(|| AccountPolicy::for_asset_class(ac).default_starting_cash);
                (ac, Mutex::new(PaperAccount::new(ac, cash)))
            })
            .collect();
        info!(
            asset_classes = ALL_ASSET_CLASSES.len(),
            "paper trading engine initialised — all accounts internal, no external venue APIs"
        );
        Self {
            accounts,
            marks: DashMap::new(),
            orders: DashMap::new(),
            resting: DashMap::new(),
            idempotency: DashMap::new(),
            terminal_fifo: Mutex::new(VecDeque::new()),
            sims,
        }
    }

    /// A [`super::PaperBroker`] view of this engine for one asset class.
    pub fn broker(self: &Arc<Self>, asset_class: AssetClass) -> super::PaperBroker {
        super::PaperBroker::for_engine(Arc::clone(self), asset_class)
    }

    // ── Mark-price board ─────────────────────────────────────────────────────

    /// Record the latest mark for `instrument_id` and fill any resting orders
    /// it crosses.  Called by the hot-path bar builder on every tick.
    pub fn on_mark(&self, instrument_id: &str, mark: Price) {
        if let Some(mut entry) = self.marks.get_mut(instrument_id) {
            *entry = mark;
        } else {
            self.marks.insert(instrument_id.to_owned(), mark);
        }
        // Fast path: nothing resting on this instrument.
        let resting_ids: Vec<String> = match self.resting.get(instrument_id) {
            Some(ids) if !ids.is_empty() => ids.clone(),
            _ => return,
        };
        self.fill_resting(instrument_id, mark, &resting_ids);
    }

    /// Latest mark for an instrument, if one has been observed.
    pub fn mark(&self, instrument_id: &str) -> Option<Price> {
        self.marks.get(instrument_id).map(|p| *p)
    }

    // ── Order lifecycle ──────────────────────────────────────────────────────

    /// Execute an order intent against the internal account for `asset_class`.
    ///
    /// Market orders fill immediately at the simulated price; limit orders fill
    /// when marketable, otherwise rest (GTC/Day) or cancel (IOC/FOK).
    /// Resubmitting an idempotency key already seen returns the original
    /// order id without executing anything.
    pub fn submit(
        &self,
        asset_class: AssetClass,
        intent: &OrderIntent,
    ) -> Result<String, PaperTradeError> {
        if let Some(existing) = self.idempotency.get(&intent.idempotency_key) {
            debug!(order_id = %existing.value(), "duplicate paper submit ignored");
            return Ok(existing.value().clone());
        }

        let order_id = Uuid::new_v4().to_string();
        let mut record = PaperOrderRecord::new(order_id.clone(), asset_class, intent.clone());

        let Some(mark) = self.mark(&intent.instrument_id) else {
            record.state = BrokerOrderState::Rejected;
            let reason = format!("no mark price for {}", intent.instrument_id);
            self.account(asset_class)
                .lock()
                .expect("paper account lock")
                .record_rejection(&order_id, &intent.instrument_id, &reason);
            self.insert_record(record);
            return Err(PaperTradeError::NoMarkPrice {
                instrument_id: intent.instrument_id.clone(),
            });
        };

        let fill = self
            .sims
            .for_structure(asset_class.market_structure())
            .simulate_fill(intent, mark);

        if fill.filled_qty > Decimal::ZERO {
            let applied = {
                let mut account = self
                    .account(asset_class)
                    .lock()
                    .expect("paper account lock");
                let result =
                    account.apply_fill(&order_id, &fill, &|id| self.mark(id).map(Price::inner));
                if let Err(ref e) = result {
                    account.record_rejection(&order_id, &intent.instrument_id, &e.to_string());
                }
                result
            };
            match applied {
                Ok(()) => record.record_fill(&fill),
                Err(e) => {
                    record.state = BrokerOrderState::Rejected;
                    self.insert_record(record);
                    return Err(e);
                }
            }
        }

        // Decide what happens to any unfilled remainder.
        let mut rests = false;
        if record.filled_qty < intent.size.inner() {
            rests = matches!(intent.order_type, OrderType::Limit | OrderType::StopLimit)
                && matches!(intent.time_in_force, TimeInForce::Gtc | TimeInForce::Day);
            if !rests {
                // Unfilled IOC/FOK, or a partially-filled taker remainder: kill it.
                record.state = BrokerOrderState::Cancelled;
            }
            // When resting, state stays New / PartiallyFilled until the mark crosses.
        }

        debug!(
            %order_id,
            instrument_id = %intent.instrument_id,
            state = ?record.state,
            filled = %record.filled_qty,
            "paper order processed"
        );
        // Insert the record before indexing it as resting so a concurrent
        // on_mark never sees a resting id without a backing record.
        self.insert_record(record);
        if rests {
            self.resting
                .entry(intent.instrument_id.clone())
                .or_default()
                .push(order_id.clone());
        }
        Ok(order_id)
    }

    /// Cancel an open paper order.  Cancelling an already-terminal order is a
    /// no-op; an unknown id is an error.
    pub fn cancel(&self, order_id: &str) -> Result<(), PaperTradeError> {
        let instrument_id = {
            let mut record = self
                .orders
                .get_mut(order_id)
                .ok_or_else(|| PaperTradeError::UnknownOrder(order_id.to_owned()))?;
            if !record.is_open() {
                return Ok(());
            }
            record.state = BrokerOrderState::Cancelled;
            record.updated_at = Utc::now();
            record.intent.instrument_id.clone()
        };
        self.remove_from_resting(&instrument_id, &[order_id.to_owned()]);
        self.push_terminal(order_id.to_owned());
        Ok(())
    }

    /// Status of any order the engine still retains.
    pub fn order_status(&self, order_id: &str) -> Option<BrokerOrderStatus> {
        self.orders.get(order_id).map(|r| r.to_status())
    }

    /// All open (resting / partially filled) orders, optionally per asset class.
    pub fn open_orders(&self, asset_class: Option<AssetClass>) -> Vec<BrokerOrderStatus> {
        self.orders
            .iter()
            .filter(|r| r.is_open() && asset_class.is_none_or(|ac| r.asset_class == ac))
            .map(|r| r.to_status())
            .collect()
    }

    // ── Account views ────────────────────────────────────────────────────────

    /// Broker-shaped positions for one asset class.
    pub fn positions(&self, asset_class: AssetClass) -> Vec<BrokerPosition> {
        let account = self
            .account(asset_class)
            .lock()
            .expect("paper account lock");
        let mut out: Vec<BrokerPosition> = account
            .positions()
            .iter()
            .map(|(id, p)| BrokerPosition {
                instrument_id: id.clone(),
                quantity: p.quantity,
                avg_entry_price: Price::from_decimal(p.average_entry_price),
            })
            .collect();
        out.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
        out
    }

    /// Account summary (cash, equity, margin, positions) for one asset class.
    pub fn snapshot(&self, asset_class: AssetClass) -> PaperAccountSnapshot {
        self.account(asset_class)
            .lock()
            .expect("paper account lock")
            .snapshot(&|id| self.mark(id).map(Price::inner))
    }

    /// Snapshots for every asset class, in policy-table order.
    pub fn snapshots(&self) -> Vec<PaperAccountSnapshot> {
        ALL_ASSET_CLASSES
            .into_iter()
            .map(|ac| self.snapshot(ac))
            .collect()
    }

    /// Journal entries for one asset class at or after `since`.
    pub fn transactions_since(
        &self,
        asset_class: AssetClass,
        since: Option<DateTime<Utc>>,
    ) -> Vec<PaperLedgerEntry> {
        self.account(asset_class)
            .lock()
            .expect("paper account lock")
            .transactions_since(since)
    }

    // ── Internal admin operations (settlement / funding) ─────────────────────

    /// Resolve a binary prediction-market contract held by the paper account.
    pub fn settle_binary(
        &self,
        instrument_id: &str,
        won: bool,
    ) -> Result<Decimal, PaperTradeError> {
        self.account(AssetClass::PredictionMarket)
            .lock()
            .expect("paper account lock")
            .settle_binary(instrument_id, won)
    }

    /// Close a position at a settlement price (futures / option expiry).
    pub fn settle_at_price(
        &self,
        asset_class: AssetClass,
        instrument_id: &str,
        price: Decimal,
    ) -> Result<Decimal, PaperTradeError> {
        self.account(asset_class)
            .lock()
            .expect("paper account lock")
            .settle_position(instrument_id, price)
    }

    /// Charge a perpetual-swap funding payment at the current mark.
    pub fn apply_funding(
        &self,
        instrument_id: &str,
        rate: Decimal,
    ) -> Result<Decimal, PaperTradeError> {
        let mark = self
            .mark(instrument_id)
            .ok_or_else(|| PaperTradeError::NoMarkPrice {
                instrument_id: instrument_id.to_owned(),
            })?;
        self.account(AssetClass::PerpetualSwap)
            .lock()
            .expect("paper account lock")
            .apply_funding(instrument_id, mark.inner(), rate)
    }

    // ── Internals ────────────────────────────────────────────────────────────

    fn account(&self, asset_class: AssetClass) -> &Mutex<PaperAccount> {
        self.accounts
            .get(&asset_class)
            .expect("every asset class has a paper account")
    }

    /// Try to fill resting orders on `instrument_id` at the new mark.
    fn fill_resting(&self, instrument_id: &str, mark: Price, resting_ids: &[String]) {
        let mut done: Vec<String> = Vec::new();
        for order_id in resting_ids {
            // Hold the order entry while applying so cancel() cannot interleave
            // between the marketability check and the account update.
            let Some(mut record) = self.orders.get_mut(order_id) else {
                done.push(order_id.clone());
                continue;
            };
            if !record.is_open() {
                done.push(order_id.clone());
                continue;
            }
            let remaining = record.intent.size.inner() - record.filled_qty;
            if remaining <= Decimal::ZERO {
                done.push(order_id.clone());
                continue;
            }
            let mut probe = record.intent.clone();
            probe.size = Size::from_decimal(remaining);
            let fill = self
                .sims
                .for_structure(record.asset_class.market_structure())
                .simulate_fill(&probe, mark);
            if fill.filled_qty <= Decimal::ZERO {
                continue; // still not marketable — keep resting
            }
            let applied = {
                let mut account = self
                    .account(record.asset_class)
                    .lock()
                    .expect("paper account lock");
                let result =
                    account.apply_fill(order_id, &fill, &|id| self.mark(id).map(Price::inner));
                if let Err(ref e) = result {
                    account.record_rejection(order_id, instrument_id, &e.to_string());
                }
                result
            };
            match applied {
                Ok(()) => {
                    record.record_fill(&fill);
                    if record.state == BrokerOrderState::Filled {
                        done.push(order_id.clone());
                        drop(record);
                        self.push_terminal(order_id.clone());
                    }
                }
                Err(e) => {
                    debug!(%order_id, error = %e, "resting paper order rejected at fill time");
                    record.state = BrokerOrderState::Rejected;
                    record.updated_at = Utc::now();
                    done.push(order_id.clone());
                    drop(record);
                    self.push_terminal(order_id.clone());
                }
            }
        }
        if !done.is_empty() {
            self.remove_from_resting(instrument_id, &done);
        }
    }

    fn remove_from_resting(&self, instrument_id: &str, order_ids: &[String]) {
        if let Some(mut ids) = self.resting.get_mut(instrument_id) {
            ids.retain(|id| !order_ids.contains(id));
        }
        self.resting
            .remove_if(instrument_id, |_, ids| ids.is_empty());
    }

    fn insert_record(&self, record: PaperOrderRecord) {
        self.idempotency
            .insert(record.intent.idempotency_key, record.order_id.clone());
        let terminal = !record.is_open();
        let order_id = record.order_id.clone();
        self.orders.insert(order_id.clone(), record);
        if terminal {
            self.push_terminal(order_id);
        }
    }

    /// Track a terminal order and evict the oldest ones beyond capacity.
    fn push_terminal(&self, order_id: String) {
        let evicted: Vec<String> = {
            let mut fifo = self.terminal_fifo.lock().expect("terminal fifo lock");
            fifo.push_back(order_id);
            let mut evicted = Vec::new();
            while fifo.len() > ORDER_HISTORY_CAPACITY {
                if let Some(old) = fifo.pop_front() {
                    evicted.push(old);
                }
            }
            evicted
        };
        for old in evicted {
            if let Some((_, record)) = self.orders.remove(&old) {
                self.idempotency.remove(&record.intent.idempotency_key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::Side;
    use rust_decimal_macros::dec;

    fn engine() -> PaperTradingEngine {
        PaperTradingEngine::new()
    }

    fn market(instrument: &str, side: Side, qty: Decimal) -> OrderIntent {
        OrderIntent::new(
            instrument,
            side,
            OrderType::Market,
            Size::from_decimal(qty),
            None,
            None,
        )
    }

    fn limit(instrument: &str, side: Side, qty: Decimal, price: Decimal) -> OrderIntent {
        OrderIntent::new(
            instrument,
            side,
            OrderType::Limit,
            Size::from_decimal(qty),
            Some(Price::from_decimal(price)),
            None,
        )
    }

    #[test]
    fn submit_without_mark_is_rejected_but_recorded() {
        let eng = engine();
        let intent = market("BTC-USD", Side::Buy, dec!(1));
        let err = eng.submit(AssetClass::CryptoSpotCex, &intent).unwrap_err();
        assert!(matches!(err, PaperTradeError::NoMarkPrice { .. }));
        // The rejected order is queryable and the resubmit is a no-op.
        let again = eng.submit(AssetClass::CryptoSpotCex, &intent).unwrap();
        let status = eng.order_status(&again).unwrap();
        assert_eq!(status.state, BrokerOrderState::Rejected);
    }

    #[test]
    fn market_order_fills_and_updates_account() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        let id = eng
            .submit(
                AssetClass::CryptoSpotCex,
                &market("BTC-USD", Side::Buy, dec!(1)),
            )
            .unwrap();
        let status = eng.order_status(&id).unwrap();
        assert_eq!(status.state, BrokerOrderState::Filled);
        assert!(status.avg_fill_price.is_some());

        let positions = eng.positions(AssetClass::CryptoSpotCex);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].quantity, dec!(1));

        let snap = eng.snapshot(AssetClass::CryptoSpotCex);
        assert!(snap.cash < dec!(100_000), "cash must be debited");
    }

    #[test]
    fn resubmit_same_idempotency_key_is_noop() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        let intent = market("BTC-USD", Side::Buy, dec!(1));
        let first = eng.submit(AssetClass::CryptoSpotCex, &intent).unwrap();
        let second = eng.submit(AssetClass::CryptoSpotCex, &intent).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            eng.positions(AssetClass::CryptoSpotCex)[0].quantity,
            dec!(1)
        );
    }

    #[test]
    fn nonmarketable_limit_rests_then_fills_when_mark_crosses() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        let id = eng
            .submit(
                AssetClass::CryptoSpotCex,
                &limit("BTC-USD", Side::Buy, dec!(1), dec!(49_000)),
            )
            .unwrap();
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
        assert_eq!(eng.open_orders(Some(AssetClass::CryptoSpotCex)).len(), 1);

        // Mark drops through the limit — the resting order fills.
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(48_900)));
        let status = eng.order_status(&id).unwrap();
        assert_eq!(status.state, BrokerOrderState::Filled);
        assert_eq!(status.avg_fill_price.unwrap().inner(), dec!(49_000));
        assert!(eng.open_orders(None).is_empty());
        assert_eq!(
            eng.positions(AssetClass::CryptoSpotCex)[0].quantity,
            dec!(1)
        );
    }

    #[test]
    fn cancel_removes_resting_order() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        let id = eng
            .submit(
                AssetClass::CryptoSpotCex,
                &limit("BTC-USD", Side::Buy, dec!(1), dec!(40_000)),
            )
            .unwrap();
        eng.cancel(&id).unwrap();
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Cancelled
        );
        // The cancelled order never fills, even if the mark crosses.
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(39_000)));
        assert!(eng.positions(AssetClass::CryptoSpotCex).is_empty());
        // Cancelling again is a no-op; unknown ids error.
        eng.cancel(&id).unwrap();
        assert!(eng.cancel("nope").is_err());
    }

    #[test]
    fn ioc_limit_away_from_market_cancels_immediately() {
        let eng = engine();
        eng.on_mark("AAPL", Price::from_decimal(dec!(200)));
        let mut intent = limit("AAPL", Side::Buy, dec!(10), dec!(150));
        intent.time_in_force = TimeInForce::Ioc;
        let id = eng.submit(AssetClass::Equity, &intent).unwrap();
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Cancelled
        );
        assert!(eng.open_orders(None).is_empty());
    }

    #[test]
    fn accounts_are_isolated_per_asset_class() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        eng.on_mark("AAPL", Price::from_decimal(dec!(200)));
        eng.submit(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Buy, dec!(1)),
        )
        .unwrap();
        eng.submit(AssetClass::Equity, &market("AAPL", Side::Buy, dec!(10)))
            .unwrap();

        assert_eq!(eng.positions(AssetClass::CryptoSpotCex).len(), 1);
        assert_eq!(eng.positions(AssetClass::Equity).len(), 1);
        // Equity cash untouched by the BTC trade.
        let equity_snap = eng.snapshot(AssetClass::Equity);
        assert!(equity_snap.cash > dec!(97_000) && equity_snap.cash < dec!(100_000));
    }

    #[test]
    fn oversized_order_is_rejected_and_account_unchanged() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        let err = eng
            .submit(
                AssetClass::CryptoSpotCex,
                &market("BTC-USD", Side::Buy, dec!(100)),
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::InsufficientCash { .. }));
        let snap = eng.snapshot(AssetClass::CryptoSpotCex);
        assert_eq!(snap.cash, dec!(100_000));
        assert!(snap.positions.is_empty());
    }

    #[test]
    fn resting_order_rejected_at_fill_time_when_funds_are_gone() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        // Rest a buy that will cost ~98k when it triggers.
        let resting_id = eng
            .submit(
                AssetClass::CryptoSpotCex,
                &limit("BTC-USD", Side::Buy, dec!(2), dec!(49_000)),
            )
            .unwrap();
        // Spend most of the cash first.
        eng.submit(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Buy, dec!(1)),
        )
        .unwrap();
        // Trigger the resting order — it can no longer be afforded.
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(48_000)));
        assert_eq!(
            eng.order_status(&resting_id).unwrap().state,
            BrokerOrderState::Rejected
        );
    }

    #[test]
    fn perp_funding_and_binary_settlement_work_through_engine() {
        let eng = engine();
        eng.on_mark("BTC-PERP", Price::from_decimal(dec!(50_000)));
        eng.submit(
            AssetClass::PerpetualSwap,
            &market("BTC-PERP", Side::Buy, dec!(1)),
        )
        .unwrap();
        let payment = eng.apply_funding("BTC-PERP", dec!(0.0001)).unwrap();
        assert!(payment > Decimal::ZERO);

        eng.on_mark("KX-EVENT", Price::from_decimal(dec!(0.40)));
        eng.submit(
            AssetClass::PredictionMarket,
            &market("KX-EVENT", Side::Buy, dec!(100)),
        )
        .unwrap();
        let realized = eng.settle_binary("KX-EVENT", true).unwrap();
        assert!(realized > Decimal::ZERO);
    }
}

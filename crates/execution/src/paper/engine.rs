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
///
/// CLOB asset classes additionally carry per-class overrides so futures, FX,
/// and perpetuals fill with their own tick sizes, spreads, and fee schedules
/// instead of inheriting crypto-spot defaults.
#[derive(Debug)]
pub struct SimulatorSet {
    /// Default CLOB simulator (crypto spot CEX defaults).
    pub clob: ClobFillSimulator,
    /// Per-asset-class CLOB tuning (futures / FX / perps).
    pub clob_overrides: HashMap<AssetClass, ClobFillSimulator>,
    /// Default broker-quote simulator (equity defaults).
    pub broker_quote: BrokerQuoteFillSimulator,
    /// Per-asset-class broker-quote tuning (options trade much wider; bonds
    /// carry wider spread and a nominal commission).
    pub broker_quote_overrides: HashMap<AssetClass, BrokerQuoteFillSimulator>,
    /// Default AMM simulator (DEX spot: Uniswap v2-style 0.3% fee + impact).
    pub amm: AmmQuoteSwapSimulator,
    /// Per-asset-class AMM overrides (NFT trades carry royalty + platform fees
    /// on top of gas, significantly higher than DEX spot).
    pub amm_overrides: HashMap<AssetClass, AmmQuoteSwapSimulator>,
    pub prediction: PredictionMarketFillSimulator,
}

impl Default for SimulatorSet {
    fn default() -> Self {
        Self::realistic()
    }
}


impl SimulatorSet {
    /// Per-asset-class tuned simulators approximating real venue economics:
    /// - FX majors: sub-pip spreads, pip tick, no per-trade commission
    ///   (spread-only retail pricing), deep books.
    /// - Listed futures: index-style 0.25 tick, tight spread, small
    ///   per-notional commission.
    /// - Perpetual swaps: 0.1 tick, taker fee ~5 bps.
    /// - Crypto spot keeps its defaults but gains a size-impact curve.
    /// - Options: ~1% half-spread (orders of magnitude wider than equities).
    /// - Bonds: wider NBBO (~25 bps) + $1/bond nominal commission.
    /// - DEX spot: default 0.3% impact + 0.3% protocol fee + $2 gas.
    /// - NFT: 0.5% price impact, 2.5% combined platform+royalty fees, $15 gas.
    ///
    /// All CLOB classes carry a linear size-impact model capped per class, so
    /// a $5M market order pays visibly more than a $5k one.
    pub fn realistic() -> Self {
        use rust_decimal_macros::dec;
        let mut clob_overrides = HashMap::new();
        clob_overrides.insert(
            AssetClass::CryptoSpotCex,
            ClobFillSimulator {
                depth_notional: Some(dec!(1_000_000)),
                impact_bps_at_depth: dec!(5),
                max_impact_bps: dec!(50),
                ..ClobFillSimulator::default()
            },
        );
        clob_overrides.insert(
            AssetClass::Fx,
            ClobFillSimulator {
                half_spread_bps: dec!(0.2),
                slippage_ticks: Decimal::ONE,
                tick_size: dec!(0.0001),
                fee_rate: Decimal::ZERO,
                partial_fill_ratio: Decimal::ONE,
                depth_notional: Some(dec!(5_000_000)),
                impact_bps_at_depth: dec!(1),
                max_impact_bps: dec!(20),
            },
        );
        clob_overrides.insert(
            AssetClass::FuturesExpiring,
            ClobFillSimulator {
                half_spread_bps: dec!(1),
                slippage_ticks: Decimal::ONE,
                tick_size: dec!(0.25),
                fee_rate: dec!(0.00005), // ~0.5 bps per side
                partial_fill_ratio: Decimal::ONE,
                depth_notional: Some(dec!(2_000_000)),
                impact_bps_at_depth: dec!(2),
                max_impact_bps: dec!(30),
            },
        );
        clob_overrides.insert(
            AssetClass::PerpetualSwap,
            ClobFillSimulator {
                half_spread_bps: dec!(1),
                slippage_ticks: Decimal::ONE,
                tick_size: dec!(0.1),
                fee_rate: dec!(0.0005), // 5 bps taker
                partial_fill_ratio: Decimal::ONE,
                depth_notional: Some(dec!(1_000_000)),
                impact_bps_at_depth: dec!(5),
                max_impact_bps: dec!(50),
            },
        );

        let mut broker_quote_overrides = HashMap::new();
        broker_quote_overrides.insert(
            AssetClass::Option,
            BrokerQuoteFillSimulator {
                half_spread_bps: dec!(100),  // ~1% of premium — options trade wide
                commission_flat: dec!(0.65), // typical per-contract commission
                ..BrokerQuoteFillSimulator::default()
            },
        );
        // Bonds have wider NBBO than equities (~25 bps retail) and nominal commission.
        broker_quote_overrides.insert(
            AssetClass::Bond,
            BrokerQuoteFillSimulator {
                half_spread_bps: dec!(25),   // 25 bps — typical retail bond spread
                commission_flat: dec!(1),    // $1/bond nominal commission
                ..BrokerQuoteFillSimulator::default()
            },
        );

        // NFT trades: 0.5% floor-price impact + 2.5% platform/royalty fee + 0.005 ETH gas.
        // Gas is denominated in ETH (the NFT account's quote currency), not USD.
        // 0.005 ETH ≈ $15 at $3 000/ETH — higher than DEX due to heavier on-chain writes.
        let mut amm_overrides = HashMap::new();
        amm_overrides.insert(
            AssetClass::Nft,
            AmmQuoteSwapSimulator {
                price_impact_bps: dec!(50),    // 0.5% — floor price is volatile
                protocol_fee_bps: dec!(250),   // 2.5% combined platform + royalty fees
                flat_fee: dec!(0.005),         // 0.005 ETH gas per NFT transaction
            },
        );

        Self {
            clob: ClobFillSimulator::default(),
            clob_overrides,
            broker_quote: BrokerQuoteFillSimulator::default(),
            broker_quote_overrides,
            amm: AmmQuoteSwapSimulator::default(), // DEX: 30 bps impact + 0.3% fee + $2 gas
            amm_overrides,
            prediction: PredictionMarketFillSimulator::default(),
        }
    }

    fn for_asset_class(&self, asset_class: AssetClass) -> &dyn PaperFillSimulator {
        match asset_class.market_structure() {
            MarketStructure::Clob => self.clob_overrides.get(&asset_class).unwrap_or(&self.clob),
            MarketStructure::BrokerQuote => self
                .broker_quote_overrides
                .get(&asset_class)
                .unwrap_or(&self.broker_quote),
            MarketStructure::AmmSwap => self.amm_overrides.get(&asset_class).unwrap_or(&self.amm),
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

/// Latest observed mark for one instrument.
#[derive(Clone, Copy, Debug)]
struct MarkEntry {
    price: Price,
    at: DateTime<Utc>,
}

/// Realism gates applied at submit time.
///
/// Both gates default **off** in [`PaperTradingEngine::new`] so unit tests
/// stay deterministic at any wall-clock time; the platform binary uses
/// [`PaperEngineConfig::realistic`] so paper orders face the same calendar
/// and data-freshness constraints a live venue would impose.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaperEngineConfig {
    /// Reject orders submitted while the asset class's market session is
    /// closed (see [`super::session`]).
    pub enforce_sessions: bool,
    /// Reject fills when the latest mark is older than this many seconds.
    pub max_mark_age_secs: Option<i64>,
}

impl PaperEngineConfig {
    /// Production gates: sessions enforced, marks must be < 5 minutes old.
    pub fn realistic() -> Self {
        Self {
            enforce_sessions: true,
            max_mark_age_secs: Some(300),
        }
    }
}

/// Internal paper execution venue covering every asset class.
pub struct PaperTradingEngine {
    accounts: HashMap<AssetClass, Mutex<PaperAccount>>,
    /// Latest mark (price + observation time) per instrument id.
    marks: DashMap<String, MarkEntry>,
    /// Instrument id → asset class, registered by the data pipelines, so the
    /// multi-asset broker can route any instrument to its account.
    instrument_classes: DashMap<String, AssetClass>,
    /// Every order the engine has seen, keyed by engine order id.
    orders: DashMap<String, PaperOrderRecord>,
    /// Resting (open limit) order ids per instrument id.
    resting: DashMap<String, Vec<String>>,
    /// Submitted idempotency keys → engine order id (resubmit is a no-op).
    idempotency: DashMap<Uuid, String>,
    /// FIFO of terminal order ids for bounded history eviction.
    terminal_fifo: Mutex<VecDeque<String>>,
    sims: SimulatorSet,
    config: PaperEngineConfig,
}

impl Default for PaperTradingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PaperTradingEngine {
    /// Engine with default policies and simulators; every asset class gets an
    /// account seeded with its policy's default starting cash.  Realism gates
    /// (sessions, mark freshness) are off — see [`Self::realistic`].
    pub fn new() -> Self {
        Self::with_config(
            &HashMap::new(),
            SimulatorSet::default(),
            PaperEngineConfig::default(),
        )
    }

    /// Production engine: tuned per-class simulators plus session and
    /// mark-freshness enforcement.
    pub fn realistic() -> Self {
        Self::with_config(
            &HashMap::new(),
            SimulatorSet::realistic(),
            PaperEngineConfig::realistic(),
        )
    }

    /// Engine with per-class starting-cash overrides, custom simulators, and
    /// realism gates.
    pub fn with_config(
        starting_cash_overrides: &HashMap<AssetClass, Decimal>,
        sims: SimulatorSet,
        config: PaperEngineConfig,
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
            instrument_classes: DashMap::new(),
            orders: DashMap::new(),
            resting: DashMap::new(),
            idempotency: DashMap::new(),
            terminal_fifo: Mutex::new(VecDeque::new()),
            sims,
            config,
        }
    }

    /// A [`super::PaperBroker`] view of this engine for one asset class.
    pub fn broker(self: &Arc<Self>, asset_class: AssetClass) -> super::PaperBroker {
        super::PaperBroker::for_engine(Arc::clone(self), asset_class)
    }

    /// A [`super::MultiAssetPaperBroker`] that routes each order to the
    /// account of its instrument's registered asset class.
    pub fn multi_asset_broker(self: &Arc<Self>) -> super::MultiAssetPaperBroker {
        super::MultiAssetPaperBroker::for_engine(Arc::clone(self))
    }

    // ── Instrument registry ──────────────────────────────────────────────────

    /// Register which asset class an instrument belongs to.  Called once by
    /// each data pipeline at spawn; idempotent.
    pub fn register_instrument(&self, instrument_id: &str, asset_class: AssetClass) {
        self.instrument_classes
            .insert(instrument_id.to_owned(), asset_class);
    }

    /// Asset class an instrument was registered under, if any.
    pub fn asset_class_of(&self, instrument_id: &str) -> Option<AssetClass> {
        self.instrument_classes.get(instrument_id).map(|e| *e)
    }

    // ── Mark-price board ─────────────────────────────────────────────────────

    /// Record the latest mark for `instrument_id` and fill any resting orders
    /// it crosses.  Called by the hot-path bar builder on every tick.
    pub fn on_mark(&self, instrument_id: &str, mark: Price) {
        let entry = MarkEntry {
            price: mark,
            at: Utc::now(),
        };
        if let Some(mut existing) = self.marks.get_mut(instrument_id) {
            *existing = entry;
        } else {
            self.marks.insert(instrument_id.to_owned(), entry);
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
        self.marks.get(instrument_id).map(|e| e.price)
    }

    /// Latest mark and its observation time.
    pub fn mark_with_time(&self, instrument_id: &str) -> Option<(Price, DateTime<Utc>)> {
        self.marks.get(instrument_id).map(|e| (e.price, e.at))
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
        self.submit_at(asset_class, intent, Utc::now())
    }

    /// [`Self::submit`] with an explicit clock — the realism gates (session
    /// calendar, mark freshness) are evaluated against `now`.
    pub fn submit_at(
        &self,
        asset_class: AssetClass,
        intent: &OrderIntent,
        now: DateTime<Utc>,
    ) -> Result<String, PaperTradeError> {
        if let Some(existing) = self.idempotency.get(&intent.idempotency_key) {
            debug!(order_id = %existing.value(), "duplicate paper submit ignored");
            return Ok(existing.value().clone());
        }

        let order_id = Uuid::new_v4().to_string();
        let mut record = PaperOrderRecord::new(order_id.clone(), asset_class, intent.clone());

        // Session gate: a live venue would reject this order outright.
        if self.config.enforce_sessions && !super::session::is_open(asset_class, now) {
            record.state = BrokerOrderState::Rejected;
            let reason = format!("market closed for {}", asset_class.as_str());
            self.account(asset_class)
                .lock()
                .expect("paper account lock")
                .record_rejection(&order_id, &intent.instrument_id, &reason);
            self.insert_record(record);
            return Err(PaperTradeError::MarketClosed { asset_class });
        }

        let Some((mark, mark_at)) = self.mark_with_time(&intent.instrument_id) else {
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

        // Freshness gate: never fill against data a live venue wouldn't show.
        if let Some(max_age) = self.config.max_mark_age_secs {
            let age = (now - mark_at).num_seconds();
            if age > max_age {
                record.state = BrokerOrderState::Rejected;
                let reason = format!(
                    "stale mark for {} ({age}s old, max {max_age}s)",
                    intent.instrument_id
                );
                self.account(asset_class)
                    .lock()
                    .expect("paper account lock")
                    .record_rejection(&order_id, &intent.instrument_id, &reason);
                self.insert_record(record);
                return Err(PaperTradeError::StaleMark {
                    instrument_id: intent.instrument_id.clone(),
                    age_secs: age,
                });
            }
        }

        let fill = self
            .sims
            .for_asset_class(asset_class)
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
                .for_asset_class(record.asset_class)
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
    fn session_gate_rejects_equity_orders_on_weekends() {
        use chrono::TimeZone;
        let eng = PaperTradingEngine::with_config(
            &HashMap::new(),
            SimulatorSet::default(),
            PaperEngineConfig {
                enforce_sessions: true,
                max_mark_age_secs: None,
            },
        );
        eng.on_mark("AAPL", Price::from_decimal(dec!(200)));
        let saturday = Utc.with_ymd_and_hms(2026, 6, 13, 18, 0, 0).unwrap();
        let err = eng
            .submit_at(
                AssetClass::Equity,
                &market("AAPL", Side::Buy, dec!(1)),
                saturday,
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::MarketClosed { .. }));

        // Crypto trades through the same weekend instant.
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        eng.submit_at(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Buy, dec!(1)),
            saturday,
        )
        .unwrap();

        // And the same equity order succeeds during Monday RTH.
        let monday_rth = Utc.with_ymd_and_hms(2026, 6, 15, 15, 0, 0).unwrap();
        eng.submit_at(
            AssetClass::Equity,
            &market("AAPL", Side::Buy, dec!(1)),
            monday_rth,
        )
        .unwrap();
    }

    #[test]
    fn freshness_gate_rejects_stale_marks() {
        let eng = PaperTradingEngine::with_config(
            &HashMap::new(),
            SimulatorSet::default(),
            PaperEngineConfig {
                enforce_sessions: false,
                max_mark_age_secs: Some(300),
            },
        );
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        // Fresh mark fills.
        eng.submit(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Buy, dec!(1)),
        )
        .unwrap();
        // Pretend ten minutes pass with no new ticks.
        let later = Utc::now() + chrono::Duration::seconds(600);
        let err = eng
            .submit_at(
                AssetClass::CryptoSpotCex,
                &market("BTC-USD", Side::Sell, dec!(1)),
                later,
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::StaleMark { .. }));
    }

    #[test]
    fn per_asset_class_clob_tuning_applies() {
        let eng = engine();
        eng.on_mark("EUR-USD", Price::from_decimal(dec!(1.1000)));
        let id = eng
            .submit(AssetClass::Fx, &market("EUR-USD", Side::Buy, dec!(10_000)))
            .unwrap();
        let fx_fill = eng
            .order_status(&id)
            .unwrap()
            .avg_fill_price
            .unwrap()
            .inner();
        // FX override: sub-pip spread + 1-pip slippage, never crypto's 5 bps +
        // penny tick.  Fill lands within ~1.3 pips of mark.
        assert!(fx_fill > dec!(1.1000) && fx_fill < dec!(1.10013));
        // Spread-only pricing: no commission charged.
        assert_eq!(eng.snapshot(AssetClass::Fx).fees_paid, Decimal::ZERO);
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

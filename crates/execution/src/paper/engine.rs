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

use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use domain::{
    instrument::{AssetClass, MarketStructure},
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side, TimeInForce},
};
use rust_decimal::Decimal;
use serde::Serialize;
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
                // Costs from `domain::fees::paper_crypto()` — the same schedule
                // object the backtest's FeeBook lists, so a fee change is one
                // edit both venues see (Set U U-2.6). The note below is why the
                // taker rate is 25 bps.
                //
                // Paper crypto mirrors the Alpaca crypto account (the v1 paper
                // venue): 0.25% taker. The CLOB default's 0.1% matched no venue
                // in this build — it was a Binance-style number, well below both
                // Alpaca (25 bps) and the Coinbase Advanced live target (40 bps+),
                // so paper P&L understated trading costs by 2.5–12×.
                depth_notional: Some(dec!(1_000_000)),
                impact_bps_at_depth: dec!(5),
                max_impact_bps: dec!(50),
                ..ClobFillSimulator::from_schedule(&domain::fees::paper_crypto())
            },
        );
        // ETFs route through the CLOB structure but trade like commission-free
        // equities (Alpaca/Schwab): no per-trade commission and a tight penny
        // spread. Without this override they inherited the crypto-spot default
        // (10 bps commission), a phantom fee no ETF venue charges.
        clob_overrides.insert(
            AssetClass::Etf,
            ClobFillSimulator {
                half_spread_bps: dec!(1), // ~1 bp — liquid ETFs quote tight
                fee_rate: Decimal::ZERO,  // commission-free
                depth_notional: Some(dec!(5_000_000)),
                impact_bps_at_depth: dec!(2),
                max_impact_bps: dec!(20),
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
                maker_fee_rate: None,
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
                maker_fee_rate: None,
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
                maker_fee_rate: None,
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
                half_spread_bps: dec!(25), // 25 bps — typical retail bond spread
                commission_flat: dec!(1),  // $1/bond nominal commission
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
                price_impact_bps: dec!(50),  // 0.5% — floor price is volatile
                protocol_fee_bps: dec!(250), // 2.5% combined platform + royalty fees
                flat_fee: dec!(0.005),       // 0.005 ETH gas per NFT transaction
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
    /// Cumulative simulated fees charged on this order.
    pub fees_paid: Decimal,
    pub state: BrokerOrderState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// A stop-type order whose trigger has traded. A triggered stop-limit rests
    /// as a limit; a triggered stop fills at once.
    pub triggered: bool,
    /// A trailing stop's best price since placement — the high for a sell stop,
    /// the low for a buy stop.
    pub best: Option<Decimal>,
    /// The deployment account the order trades in, frozen when it was
    /// submitted (Set U U-4.7); `None` for the asset class's account.
    deployment_slot: Option<usize>,
}

impl PaperOrderRecord {
    fn new(
        order_id: String,
        asset_class: AssetClass,
        intent: OrderIntent,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            order_id,
            asset_class,
            intent,
            filled_qty: Decimal::ZERO,
            avg_fill_price: None,
            fees_paid: Decimal::ZERO,
            state: BrokerOrderState::New,
            created_at: now,
            updated_at: now,
            triggered: false,
            best: None,
            deployment_slot: None,
        }
    }

    /// Whether a stop-type order's trigger is met at `price`; for a trailing
    /// stop, also moves its best price when it is not.
    ///
    /// A sell stop triggers at or below its stop, a buy stop at or above. A
    /// trailing sell stop sits `trail` below the best high seen and a trailing
    /// buy stop `trail` above the best low; the check happens **before** the best
    /// price moves, so a single print cannot both raise the stop and trigger it.
    fn trigger_met(&mut self, price: Decimal) -> Option<Decimal> {
        let side = self.intent.side;
        match self.intent.order_type {
            OrderType::Stop | OrderType::StopLimit => {
                let stop = self.intent.stop_price?.inner();
                let hit = match side {
                    Side::Sell => price <= stop,
                    Side::Buy => price >= stop,
                };
                hit.then_some(stop)
            }
            OrderType::TrailingStop => {
                let trail = self.intent.trail?.inner();
                let best = self.best.unwrap_or(price);
                let (level, hit) = match side {
                    Side::Sell => (best - trail, price <= best - trail),
                    Side::Buy => (best + trail, price >= best + trail),
                };
                if !hit {
                    self.best = Some(match side {
                        Side::Sell => best.max(price),
                        Side::Buy => best.min(price),
                    });
                }
                hit.then_some(level)
            }
            OrderType::Market | OrderType::Limit => None,
        }
    }

    /// The order as it executes once live, for `qty`: a triggered stop or
    /// trailing stop is a market order; a triggered stop-limit is a limit.
    fn live_probe(&self, qty: Decimal) -> OrderIntent {
        let mut probe = self.intent.clone();
        probe.size = Size::from_decimal(qty);
        probe.order_type = match self.intent.order_type {
            OrderType::Stop | OrderType::TrailingStop => OrderType::Market,
            OrderType::StopLimit => OrderType::Limit,
            other => other,
        };
        probe
    }

    /// Fold a fill into the record's cumulative quantity / VWAP / state.
    fn record_fill(&mut self, fill: &PaperFill, now: DateTime<Utc>) {
        let prev_notional = self
            .avg_fill_price
            .map_or(Decimal::ZERO, |p| p.inner() * self.filled_qty);
        self.filled_qty += fill.filled_qty;
        self.fees_paid += fill.fee;
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
        self.updated_at = now;
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
            // The paper engine charges the fee itself, so it always knows it.
            commission: Some(self.fees_paid),
            state: self.state.clone(),
        }
    }
}

/// Serializable, instrument-scoped view of a paper order for the trading
/// terminal (working orders + fills).
#[derive(Clone, Debug, Serialize)]
pub struct PaperOrderView {
    pub order_id: String,
    /// The intent's idempotency key -- joins this order to its `orders` row and
    /// its `fills` rows in the journal.
    pub idempotency_key: Uuid,
    pub instrument_id: String,
    pub asset_class: String,
    pub side: String,
    pub order_type: String,
    pub qty: Decimal,
    pub filled_qty: Decimal,
    pub avg_fill_price: Option<Decimal>,
    /// new | partially_filled | filled | cancelled | rejected
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PaperOrderView {
    fn from_record(r: &PaperOrderRecord) -> Self {
        let status = match &r.state {
            BrokerOrderState::New => "new",
            BrokerOrderState::PartiallyFilled => "partially_filled",
            BrokerOrderState::Filled => "filled",
            BrokerOrderState::Cancelled => "cancelled",
            BrokerOrderState::Rejected => "rejected",
            BrokerOrderState::Unknown(_) => "unknown",
        };
        let side = match r.intent.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let order_type = r.intent.order_type.as_str();
        Self {
            order_id: r.order_id.clone(),
            idempotency_key: r.intent.idempotency_key,
            instrument_id: r.intent.instrument_id.clone(),
            asset_class: r.asset_class.as_str().to_owned(),
            side: side.to_owned(),
            order_type: order_type.to_owned(),
            qty: r.intent.size.inner(),
            filled_qty: r.filled_qty,
            avg_fill_price: r.avg_fill_price.map(Price::inner),
            status: status.to_owned(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// What one attempt to fill an order did.
enum Attempt {
    Filled,
    NotMarketable,
    Rejected(PaperTradeError),
    /// Withdrawn rather than rejected: a reduce-only order with nothing left to
    /// reduce.
    Cancelled(PaperTradeError),
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
    /// Starting cash each account was seeded with, so [`Self::reset_account`]
    /// can restore the exact opening balance.
    starting_cash: HashMap<AssetClass, Decimal>,
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
    /// The time, for every stamp and every gate (Set U U-2.1).
    clock: Arc<dyn crate::clock::VenueClock>,
    /// Order ids: random for paper, sequential in a backtest.
    ids: Arc<dyn crate::clock::OrderIds>,
    /// Per-account "touched since last save" flags for [`super::persist`]
    /// (Set E #5). Set whenever an account is locked; the saver swaps it.
    dirty: HashMap<AssetClass, std::sync::atomic::AtomicBool>,
    /// One account per deployment that opened one (Set U U-4.7), so two
    /// deployments on one asset class never share cash, positions or a day's
    /// loss. Slots are filled once and never emptied, which is what lets a
    /// lock on one borrow from the engine like the asset-class accounts do.
    deployment_accounts: Vec<std::sync::OnceLock<DeploymentAccount>>,
    deployment_index: DashMap<Uuid, usize>,
    next_deployment_slot: std::sync::atomic::AtomicUsize,
}

/// How many deployment accounts one process can open.
pub const DEPLOYMENT_ACCOUNT_SLOTS: usize = 4096;

/// A deployment's own paper account.
struct DeploymentAccount {
    deployment: Uuid,
    asset_class: AssetClass,
    account: Mutex<PaperAccount>,
    dirty: std::sync::atomic::AtomicBool,
}

/// Which account an order trades in: its asset class's, or a deployment's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Book {
    Class(AssetClass),
    Deployment(usize),
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
        Self::with_clock(
            starting_cash_overrides,
            sims,
            config,
            Arc::new(crate::clock::WallClock),
            Arc::new(crate::clock::RandomIds),
        )
    }

    /// [`Self::with_config`] with an explicit clock and id source — the
    /// simulated venue's constructor (ADR-0044). Everything else about the
    /// engine is identical, which is the point.
    pub fn with_clock(
        starting_cash_overrides: &HashMap<AssetClass, Decimal>,
        sims: SimulatorSet,
        config: PaperEngineConfig,
        clock: Arc<dyn crate::clock::VenueClock>,
        ids: Arc<dyn crate::clock::OrderIds>,
    ) -> Self {
        let mut starting_cash = HashMap::new();
        let accounts = ALL_ASSET_CLASSES
            .into_iter()
            .map(|ac| {
                let cash = starting_cash_overrides
                    .get(&ac)
                    .copied()
                    .unwrap_or_else(|| AccountPolicy::for_asset_class(ac).default_starting_cash);
                starting_cash.insert(ac, cash);
                (
                    ac,
                    Mutex::new(PaperAccount::with_clock(ac, cash, Arc::clone(&clock))),
                )
            })
            .collect();
        info!(
            asset_classes = ALL_ASSET_CLASSES.len(),
            "paper trading engine initialised — all accounts internal, no external venue APIs"
        );
        Self {
            accounts,
            starting_cash,
            marks: DashMap::new(),
            instrument_classes: DashMap::new(),
            orders: DashMap::new(),
            resting: DashMap::new(),
            idempotency: DashMap::new(),
            terminal_fifo: Mutex::new(VecDeque::new()),
            dirty: ALL_ASSET_CLASSES
                .into_iter()
                .map(|ac| (ac, std::sync::atomic::AtomicBool::new(false)))
                .collect(),
            sims,
            config,
            clock,
            ids,
            deployment_accounts: (0..DEPLOYMENT_ACCOUNT_SLOTS)
                .map(|_| std::sync::OnceLock::new())
                .collect(),
            deployment_index: DashMap::new(),
            next_deployment_slot: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The engine's current time — wall or simulated.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
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
            at: self.clock.now(),
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
        self.submit_at(asset_class, intent, self.clock.now())
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

        let book = self.book_for(asset_class, intent.strategy_id.as_deref());
        if self.book_poisoned(book) {
            observability::metrics::inc(
                "paper_orders_refused_poisoned_total",
                "Paper orders refused because the account's lock was poisoned by a panic.",
                &[("asset_class", asset_class.as_str())],
            );
            return Err(PaperTradeError::AccountUnavailable { asset_class });
        }

        let order_id = self.ids.next_id();
        let mut record = PaperOrderRecord::new(order_id.clone(), asset_class, intent.clone(), now);
        record.deployment_slot = match book {
            Book::Deployment(slot) => Some(slot),
            Book::Class(_) => None,
        };

        // Session gate: a live venue would reject this order outright.
        if self.config.enforce_sessions && !super::session::is_open(asset_class, now) {
            record.state = BrokerOrderState::Rejected;
            let reason = format!("market closed for {}", asset_class.as_str());
            self.lock_book(book)
                .record_rejection(&order_id, &intent.instrument_id, &reason);
            self.insert_record(record);
            return Err(PaperTradeError::MarketClosed { asset_class });
        }

        let Some((mark, mark_at)) = self.mark_with_time(&intent.instrument_id) else {
            record.state = BrokerOrderState::Rejected;
            let reason = format!("no mark price for {}", intent.instrument_id);
            self.lock_book(book)
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
                self.lock_book(book)
                    .record_rejection(&order_id, &intent.instrument_id, &reason);
                self.insert_record(record);
                return Err(PaperTradeError::StaleMark {
                    instrument_id: intent.instrument_id.clone(),
                    age_secs: age,
                });
            }
        }

        if let Err(e) = intent.check_shape() {
            record.state = BrokerOrderState::Rejected;
            self.lock_book(book)
                .record_rejection(&order_id, &intent.instrument_id, &e.to_string());
            self.insert_record(record);
            return Err(PaperTradeError::InvalidOrder(e.to_string()));
        }
        // Post-only: a limit that would trade against the current mark is
        // refused rather than allowed to take.
        if intent.post_only {
            let limit = intent.limit_price.map(Price::inner).unwrap_or_default();
            let takes = match intent.side {
                Side::Buy => mark.inner() <= limit,
                Side::Sell => mark.inner() >= limit,
            };
            if takes {
                record.state = BrokerOrderState::Rejected;
                let e = PaperTradeError::PostOnlyWouldTake {
                    instrument_id: intent.instrument_id.clone(),
                };
                self.lock_book(book)
                    .record_rejection(&order_id, &intent.instrument_id, &e.to_string());
                self.insert_record(record);
                return Err(e);
            }
        }

        // A stop-type order is live only once its trigger has traded.
        let live = if intent.order_type.is_triggered() {
            record.best = Some(mark.inner());
            if record.trigger_met(mark.inner()).is_some() {
                record.triggered = true;
            }
            record.triggered
        } else {
            true
        };

        let mut filled_any = false;
        if live {
            // Anything that fills on arrival crossed the book: a taker fill.
            match self.attempt(&mut record, mark, domain::fees::Liquidity::Taker) {
                Attempt::Filled => filled_any = true,
                Attempt::NotMarketable => {}
                Attempt::Rejected(e) => {
                    record.state = BrokerOrderState::Rejected;
                    self.insert_record(record);
                    return Err(e);
                }
                Attempt::Cancelled(e) => {
                    record.state = BrokerOrderState::Cancelled;
                    self.lock_book(book)
                        .record_rejection(&order_id, &intent.instrument_id, &e.to_string());
                    self.insert_record(record);
                    return Err(e);
                }
            }
        }

        // Decide what happens to any unfilled remainder.
        let mut rests = false;
        if record.is_open() && record.filled_qty < intent.size.inner() {
            let resting_type = match intent.order_type {
                OrderType::Limit => true,
                // An untriggered stop waits for its trigger; a triggered
                // stop-limit waits as a limit.
                OrderType::Stop | OrderType::TrailingStop => !record.triggered,
                OrderType::StopLimit => true,
                OrderType::Market => false,
            };
            rests =
                resting_type && matches!(intent.time_in_force, TimeInForce::Gtc | TimeInForce::Day);
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
        if filled_any {
            if let Some(group) = &intent.oco_group {
                self.cancel_oco_siblings(group, &order_id);
            }
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
            record.updated_at = self.clock.now();
            record.intent.instrument_id.clone()
        };
        self.remove_from_resting(&instrument_id, &[order_id.to_owned()]);
        self.push_terminal(order_id.to_owned());
        Ok(())
    }

    /// The venue order id an idempotency key was given, once it reached the
    /// engine (accepted or rejected).
    pub fn order_id_for(&self, key: Uuid) -> Option<String> {
        self.idempotency.get(&key).map(|e| e.value().clone())
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
        self.positions_in(Book::Class(asset_class))
    }

    /// The positions of the account `strategy_id` trades in: its deployment's
    /// when it opened one (U-4.7), otherwise the asset class's.
    pub fn positions_for(&self, asset_class: AssetClass, strategy_id: Option<&str>) -> Vec<BrokerPosition> {
        self.positions_in(self.book_for(asset_class, strategy_id))
    }

    /// The snapshot of the account `strategy_id` trades in.
    pub fn snapshot_for(&self, asset_class: AssetClass, strategy_id: Option<&str>) -> PaperAccountSnapshot {
        self.snapshot_in(self.book_for(asset_class, strategy_id))
    }

    /// Today's loss on the account `strategy_id` trades in — what the risk
    /// gate's daily-loss limit reads for that strategy's orders.
    pub fn daily_loss_usd_for(&self, asset_class: AssetClass, strategy_id: Option<&str>) -> Decimal {
        self.daily_loss_in(self.book_for(asset_class, strategy_id))
    }

    fn positions_in(&self, book: Book) -> Vec<BrokerPosition> {
        let account = self.lock_book(book);
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
        self.snapshot_in(Book::Class(asset_class))
    }

    fn snapshot_in(&self, book: Book) -> PaperAccountSnapshot {
        self.lock_book(book)
            .snapshot(&|id| self.mark(id).map(Price::inner))
    }

    /// Today's loss on one account, in quote currency, positive = loss.
    ///
    /// Realized P&L for the current UTC day (net of fees) plus the unrealized
    /// P&L of everything still open — which is what COMP-002 §3.2 check 7
    /// means by "cumulative realized + unrealized loss for today".  This is the
    /// number the risk gate's daily-loss limit reads; before it existed both
    /// order doors passed `Decimal::ZERO`, so the limit could not fire.
    ///
    /// In-memory, like the whole paper account: a platform restart starts the
    /// day's tally again.
    pub fn daily_loss_usd(&self, asset_class: AssetClass) -> Decimal {
        self.daily_loss_in(Book::Class(asset_class))
    }

    fn daily_loss_in(&self, book: Book) -> Decimal {
        let realized_today = self
            .lock_book(book)
            .realized_pnl_on(self.clock.now().date_naive());
        let unrealized: Decimal = self
            .snapshot_in(book)
            .positions
            .iter()
            .map(|p| p.unrealized_pnl)
            .sum();
        -(realized_today + unrealized)
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
        self.lock_account(asset_class)
            .transactions_since(since)
    }

    /// Every order the engine retains for `instrument_id` (open and terminal),
    /// newest first.  Powers the trading-terminal working-orders and fills lists.
    pub fn orders_for_instrument(&self, instrument_id: &str) -> Vec<PaperOrderView> {
        let mut out: Vec<PaperOrderView> = self
            .orders
            .iter()
            .filter(|r| r.intent.instrument_id == instrument_id)
            .map(|r| PaperOrderView::from_record(&r))
            .collect();
        out.sort_by_key(|b| Reverse(b.created_at));
        out
    }

    // ── Reset (paper-only admin) ─────────────────────────────────────────────

    /// Reset one asset-class account to its opening state: cash and equity back
    /// to the seeded starting balance, all positions closed, and the entire
    /// transaction journal wiped.  Orders belonging to this class are purged
    /// from the engine's order/resting/idempotency maps.
    pub fn reset_account(&self, asset_class: AssetClass) {
        let cash = self
            .starting_cash
            .get(&asset_class)
            .copied()
            .unwrap_or_else(|| AccountPolicy::for_asset_class(asset_class).default_starting_cash);
        {
            let mut account = self.lock_account(asset_class);
            *account = PaperAccount::with_clock(asset_class, cash, Arc::clone(&self.clock));
        }
        // The account was replaced wholesale, so whatever a panic left half-done
        // is gone with it: the class takes orders again (Set E #8).
        self.account(asset_class).clear_poison();
        self.purge_orders(|r| r.asset_class == asset_class);
        info!(asset_class = %asset_class.as_str(), "paper account reset to opening balance");
    }

    /// Reset every paper account to its opening state and clear all order
    /// bookkeeping.  Marks and the instrument→class registry are left intact —
    /// they reflect live market data, not user funds.
    pub fn reset_all(&self) {
        for ac in ALL_ASSET_CLASSES {
            let cash = self
                .starting_cash
                .get(&ac)
                .copied()
                .unwrap_or_else(|| AccountPolicy::for_asset_class(ac).default_starting_cash);
            let mut account = self.lock_account(ac);
            *account = PaperAccount::with_clock(ac, cash, Arc::clone(&self.clock));
            drop(account);
            self.account(ac).clear_poison();
        }
        self.orders.clear();
        self.resting.clear();
        self.idempotency.clear();
        self.lock_fifo()
            .clear();
        info!("all paper accounts reset to opening balances");
    }

    /// Remove every order matching `pred` from the order store, its resting
    /// index, idempotency map, and the terminal FIFO.
    fn purge_orders(&self, pred: impl Fn(&PaperOrderRecord) -> bool) {
        let to_remove: Vec<(String, Uuid, String)> = self
            .orders
            .iter()
            .filter(|r| pred(r))
            .map(|r| {
                (
                    r.order_id.clone(),
                    r.intent.idempotency_key,
                    r.intent.instrument_id.clone(),
                )
            })
            .collect();
        for (order_id, key, instrument_id) in &to_remove {
            self.orders.remove(order_id);
            self.idempotency.remove(key);
            self.remove_from_resting(instrument_id, std::slice::from_ref(order_id));
        }
        let removed: std::collections::HashSet<&String> =
            to_remove.iter().map(|(id, _, _)| id).collect();
        self.lock_fifo()
            .retain(|id| !removed.contains(id));
    }

    // ── Internal admin operations (settlement / funding) ─────────────────────

    /// Resolve a binary prediction-market contract held by the paper account.
    pub fn settle_binary(
        &self,
        instrument_id: &str,
        won: bool,
    ) -> Result<Decimal, PaperTradeError> {
        self.settle_everywhere(AssetClass::PredictionMarket, instrument_id, |a| {
            a.settle_binary(instrument_id, won)
        })
    }

    /// Close a position at a settlement price (futures / option expiry).
    pub fn settle_at_price(
        &self,
        asset_class: AssetClass,
        instrument_id: &str,
        price: Decimal,
    ) -> Result<Decimal, PaperTradeError> {
        self.settle_everywhere(asset_class, instrument_id, |a| {
            a.settle_position(instrument_id, price)
        })
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
        self.settle_everywhere(AssetClass::PerpetualSwap, instrument_id, |a| {
            a.apply_funding(instrument_id, mark.inner(), rate)
        })
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// How much of a reduce-only order may fill now: the size of an opposite
    /// position, or nothing.
    fn reduce_capacity(&self, book: Book, instrument_id: &str, side: Side) -> Decimal {
        let pos = self.lock_book(book)
            .position(instrument_id)
            .map_or(Decimal::ZERO, |p| p.quantity);
        match side {
            Side::Buy if pos < Decimal::ZERO => -pos,
            Side::Sell if pos > Decimal::ZERO => pos,
            _ => Decimal::ZERO,
        }
    }

    /// Try to fill what remains of `record` at `mark`. Applies reduce-only
    /// capping, prices through the asset class's simulator, books the fill.
    fn attempt(
        &self,
        record: &mut PaperOrderRecord,
        mark: Price,
        liquidity: domain::fees::Liquidity,
    ) -> Attempt {
        let remaining = record.intent.size.inner() - record.filled_qty;
        if remaining <= Decimal::ZERO {
            return Attempt::NotMarketable;
        }
        let book = Self::book_of(record);
        if self.book_poisoned(book) {
            return Attempt::Rejected(PaperTradeError::AccountUnavailable {
                asset_class: record.asset_class,
            });
        }
        let mut qty = remaining;
        if record.intent.reduce_only {
            let cap = self.reduce_capacity(
                book,
                &record.intent.instrument_id,
                record.intent.side,
            );
            if cap <= Decimal::ZERO {
                return Attempt::Cancelled(PaperTradeError::ReduceOnlyWouldIncrease {
                    instrument_id: record.intent.instrument_id.clone(),
                });
            }
            qty = qty.min(cap);
        }
        let probe = record.live_probe(qty);
        let fill = self
            .sims
            .for_asset_class(record.asset_class)
            .simulate_fill_as(&probe, mark, liquidity);
        if fill.filled_qty <= Decimal::ZERO {
            return Attempt::NotMarketable;
        }
        let applied = {
            let mut account = self.lock_book(book);
            let result = account.apply_fill(&record.order_id, &fill, &|id| {
                self.mark(id).map(Price::inner)
            });
            if let Err(ref e) = result {
                account.record_rejection(
                    &record.order_id,
                    &record.intent.instrument_id,
                    &e.to_string(),
                );
            }
            result
        };
        match applied {
            Ok(()) => {
                record.record_fill(&fill, self.clock.now());
                // A reduce-only order capped at the position has nothing left to
                // reduce: its remainder is cancelled, never left to open a new
                // position later.
                if record.is_open()
                    && record.intent.reduce_only
                    && self.reduce_capacity(
                        book,
                        &record.intent.instrument_id,
                        record.intent.side,
                    ) <= Decimal::ZERO
                {
                    record.state = BrokerOrderState::Cancelled;
                }
                Attempt::Filled
            }
            Err(e) => Attempt::Rejected(e),
        }
    }

    /// Cancel every other open order in `group` — one-cancels-other.
    fn cancel_oco_siblings(&self, group: &str, filled_order_id: &str) {
        let siblings: Vec<String> = self
            .orders
            .iter()
            .filter(|r| {
                r.is_open()
                    && r.order_id != filled_order_id
                    && r.intent.oco_group.as_deref() == Some(group)
            })
            .map(|r| r.order_id.clone())
            .collect();
        for id in siblings {
            let _ = self.cancel(&id);
        }
    }

    // ── Deployment accounts (Set U U-4.7) ─────────────────────────────────────

    /// Open `deployment`'s own account, seeded with `starting_cash`. Returns
    /// `false` when it is already open (a restored account is never reseeded).
    ///
    /// # Errors
    /// Every slot is taken.
    pub fn open_deployment_account(
        &self,
        deployment: Uuid,
        asset_class: AssetClass,
        starting_cash: Decimal,
    ) -> Result<bool, String> {
        self.open_with(deployment, asset_class, || {
            PaperAccount::with_clock(asset_class, starting_cash, Arc::clone(&self.clock))
        })
    }

    fn open_with(
        &self,
        deployment: Uuid,
        asset_class: AssetClass,
        account: impl FnOnce() -> PaperAccount,
    ) -> Result<bool, String> {
        match self.deployment_index.entry(deployment) {
            dashmap::mapref::entry::Entry::Occupied(_) => Ok(false),
            dashmap::mapref::entry::Entry::Vacant(v) => {
                let slot = self
                    .next_deployment_slot
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let cell = self.deployment_accounts.get(slot).ok_or_else(|| {
                    format!("every one of the {DEPLOYMENT_ACCOUNT_SLOTS} deployment account slots is taken")
                })?;
                cell.set(DeploymentAccount {
                    deployment,
                    asset_class,
                    account: Mutex::new(account()),
                    dirty: std::sync::atomic::AtomicBool::new(true),
                })
                .map_err(|_| "a deployment account slot was filled twice".to_string())?;
                v.insert(slot);
                Ok(true)
            }
        }
    }

    /// Every deployment account, with its asset class.
    pub fn deployment_accounts(&self) -> Vec<(Uuid, AssetClass)> {
        self.deployment_accounts
            .iter()
            .filter_map(std::sync::OnceLock::get)
            .map(|d| (d.deployment, d.asset_class))
            .collect()
    }

    /// One deployment account's whole state, for persistence.
    pub fn export_deployment_account(
        &self,
        deployment: Uuid,
    ) -> Option<(AssetClass, super::account::PaperAccountState)> {
        let slot = *self.deployment_index.get(&deployment)?;
        let d = self.deployment_accounts[slot].get()?;
        Some((d.asset_class, self.lock_book(Book::Deployment(slot)).export_state()))
    }

    /// Restore a deployment account from a stored state — opening it if this
    /// process has not, replacing it if it has. Called at boot, before orders.
    ///
    /// # Errors
    /// The state names another asset class, or no slot is left.
    pub fn restore_deployment_account(
        &self,
        deployment: Uuid,
        asset_class: AssetClass,
        state: super::account::PaperAccountState,
    ) -> Result<(), String> {
        let restored = PaperAccount::from_state(asset_class, state, Arc::clone(&self.clock))?;
        let mut restored = Some(restored);
        self.open_with(deployment, asset_class, || {
            restored.take().expect("taken once")
        })?;
        if let Some(r) = restored {
            // Already open: replace it.
            let slot = *self
                .deployment_index
                .get(&deployment)
                .ok_or("deployment account vanished")?;
            *self.lock_book(Book::Deployment(slot)) = r;
        }
        if let Some(slot) = self.deployment_index.get(&deployment).map(|s| *s) {
            if let Some(d) = self.deployment_accounts[slot].get() {
                d.dirty.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        Ok(())
    }

    /// Whether a deployment account changed since the last call; clears it.
    pub fn take_deployment_dirty(&self, deployment: Uuid) -> bool {
        self.deployment_index
            .get(&deployment)
            .and_then(|slot| self.deployment_accounts[*slot].get())
            .is_some_and(|d| d.dirty.swap(false, std::sync::atomic::Ordering::AcqRel))
    }

    /// The book an order with `strategy_id` trades in: its deployment's
    /// account when it opened one on this asset class, else the class's.
    fn book_for(&self, asset_class: AssetClass, strategy_id: Option<&str>) -> Book {
        strategy_id
            .and_then(|s| Uuid::parse_str(s).ok())
            .and_then(|id| self.deployment_index.get(&id).map(|slot| *slot))
            .filter(|slot| {
                self.deployment_accounts[*slot]
                    .get()
                    .is_some_and(|d| d.asset_class == asset_class)
            })
            .map_or(Book::Class(asset_class), Book::Deployment)
    }

    fn book_of(record: &PaperOrderRecord) -> Book {
        record
            .deployment_slot
            .map_or(Book::Class(record.asset_class), Book::Deployment)
    }

    /// Lock a book, recovering poison as [`Self::lock_account`] does.
    fn lock_book(&self, book: Book) -> std::sync::MutexGuard<'_, PaperAccount> {
        match book {
            Book::Class(ac) => self.lock_account(ac),
            Book::Deployment(slot) => {
                let d = self.deployment_accounts[slot]
                    .get()
                    .expect("a book names only an opened slot");
                let guard = d.account.lock().unwrap_or_else(|poisoned| {
                    tracing::error!(
                        deployment = %d.deployment,
                        "deployment paper account mutex is poisoned — serving reads, refusing new orders"
                    );
                    poisoned.into_inner()
                });
                d.dirty.store(true, std::sync::atomic::Ordering::Release);
                guard
            }
        }
    }

    fn book_poisoned(&self, book: Book) -> bool {
        match book {
            Book::Class(ac) => self.account_poisoned(ac),
            Book::Deployment(slot) => self.deployment_accounts[slot]
                .get()
                .is_some_and(|d| d.account.is_poisoned()),
        }
    }

    /// Apply `op` to every account of `asset_class` that holds `instrument_id`
    /// — the class's and each deployment's — summing what it returns. With no
    /// holder at all, the class account answers (and so names the error).
    fn settle_everywhere(
        &self,
        asset_class: AssetClass,
        instrument_id: &str,
        op: impl Fn(&mut PaperAccount) -> Result<Decimal, PaperTradeError>,
    ) -> Result<Decimal, PaperTradeError> {
        let mut books = vec![Book::Class(asset_class)];
        books.extend(
            self.deployment_accounts
                .iter()
                .enumerate()
                .filter(|(_, c)| c.get().is_some_and(|d| d.asset_class == asset_class))
                .map(|(i, _)| Book::Deployment(i)),
        );
        let holders: Vec<Book> = books
            .into_iter()
            .filter(|b| self.lock_book(*b).position(instrument_id).is_some())
            .collect();
        if holders.is_empty() {
            return op(&mut self.lock_account(asset_class));
        }
        let mut total = Decimal::ZERO;
        for b in holders {
            total += op(&mut self.lock_book(b))?;
        }
        Ok(total)
    }

    fn account(&self, asset_class: AssetClass) -> &Mutex<PaperAccount> {
        self.accounts
            .get(&asset_class)
            .expect("every asset class has a paper account")
    }

    /// Lock one account **without panicking on poison** (Set E audit item #8).
    ///
    /// Every account access used to be `.lock().expect("paper account lock")`.
    /// One panic while a guard was held poisoned that mutex for the life of the
    /// process, after which *every* order on the class panicked its handler
    /// task — while the process kept running and looking healthy.
    ///
    /// A poisoned guard is recovered here so reads (snapshots, positions, the
    /// daily-loss figure) keep answering; **new orders are refused** on a
    /// poisoned account by [`Self::account_poisoned`] in `submit_at` and
    /// `attempt`, because its state may be mid-mutation. `reset_account` clears
    /// the poison by replacing the account.
    fn lock_account(&self, asset_class: AssetClass) -> std::sync::MutexGuard<'_, PaperAccount> {
        let guard = self.account(asset_class).lock().unwrap_or_else(|poisoned| {
            tracing::error!(
                asset_class = %asset_class.as_str(),
                "paper account mutex is poisoned — serving reads, refusing new orders"
            );
            poisoned.into_inner()
        });
        // Every mutation goes through here, so this is the persistence hook:
        // the flag is set while the guard is held, and the saver re-locks
        // before exporting, so it always sees the finished mutation.
        if let Some(flag) = self.dirty.get(&asset_class) {
            flag.store(true, std::sync::atomic::Ordering::Release);
        }
        guard
    }

    /// Whether `asset_class`'s account was touched since the last call, and
    /// clear the flag. For [`super::persist`].
    pub fn take_dirty(&self, asset_class: AssetClass) -> bool {
        self.dirty
            .get(&asset_class)
            .is_some_and(|f| f.swap(false, std::sync::atomic::Ordering::AcqRel))
    }

    /// One account's whole state, for persistence (Set E #5).
    pub fn export_account(&self, asset_class: AssetClass) -> super::account::PaperAccountState {
        self.lock_account(asset_class).export_state()
    }

    /// Replace one account with a stored state. Called at boot, before the
    /// engine takes an order; the account's poison (if any) goes with it.
    ///
    /// # Errors
    /// The state names a different asset class.
    pub fn restore_account(
        &self,
        asset_class: AssetClass,
        state: super::account::PaperAccountState,
    ) -> Result<(), String> {
        let restored = PaperAccount::from_state(asset_class, state, Arc::clone(&self.clock))?;
        *self.lock_account(asset_class) = restored;
        self.account(asset_class).clear_poison();
        // A just-restored account equals its stored row; no need to write it back.
        self.take_dirty(asset_class);
        Ok(())
    }

    /// Whether an account's mutex was poisoned by a panic. Orders are refused
    /// on such an account rather than executed against state that may be
    /// half-updated.
    #[must_use]
    pub fn account_poisoned(&self, asset_class: AssetClass) -> bool {
        self.account(asset_class).is_poisoned()
    }

    /// The terminal-order FIFO, poison-tolerant: it is a bounded eviction list
    /// with no invariant a half-finished push can break.
    fn lock_fifo(&self) -> std::sync::MutexGuard<'_, VecDeque<String>> {
        self.terminal_fifo
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Try to fill resting orders on `instrument_id` at the new mark — the
    /// paper path, one print at a time.
    fn fill_resting(&self, instrument_id: &str, mark: Price, resting_ids: &[String]) {
        let p = mark.inner();
        self.resting_pass(instrument_id, resting_ids, &|record| {
            if record.intent.order_type.is_triggered() && !record.triggered {
                // Triggered at this print: a stop fills here, a stop-limit
                // then checks its limit against the same print.
                record.trigger_met(p)?;
                record.triggered = true;
            }
            Some(mark)
        });
    }

    /// Process one closed bar's range on `instrument_id` — the simulated
    /// venue's path (ADR-0044). **Stops first:** every stop-type order is
    /// checked against the whole range before any limit is, so when a bar spans
    /// both a bracket's stop and its take-profit the stop wins and its OCO
    /// sibling is cancelled before the take-profit can fill. That is the
    /// pessimistic intrabar rule (FEAT-005 §3.4), and it is the only ordering a
    /// bar cannot contradict: the bar does not say which extreme came first.
    ///
    /// A stop that triggers inside the range fills at its own level (plus the
    /// simulator's costs); one the bar *opened* through has already filled at
    /// the open, because the driver marks the open first.
    pub fn on_bar_range(&self, instrument_id: &str, low: Price, high: Price) {
        let (lo, hi) = (low.inner(), high.inner());
        // Pass 1: stops.
        if let Some(ids) = self.resting.get(instrument_id).map(|v| v.clone()) {
            self.resting_pass(instrument_id, &ids, &|record| {
                if !record.intent.order_type.is_triggered() || record.triggered {
                    return None;
                }
                // The adverse extreme for this order's trigger.
                let probe = match record.intent.side {
                    Side::Sell => lo,
                    Side::Buy => hi,
                };
                let level = record.trigger_met(probe)?;
                record.triggered = true;
                if record.intent.order_type == OrderType::StopLimit {
                    return None; // a triggered stop-limit is a limit: pass 2
                }
                Some(Price::from_decimal(level.clamp(lo, hi)))
            });
            // A trailing stop that survived the bar ratchets to its favourable
            // extreme — after the trigger check, never before.
            for id in ids {
                if let Some(mut r) = self.orders.get_mut(&id) {
                    if r.is_open() && r.intent.order_type == OrderType::TrailingStop && !r.triggered
                    {
                        let fav = match r.intent.side {
                            Side::Sell => hi,
                            Side::Buy => lo,
                        };
                        let _ = r.trigger_met(fav);
                    }
                }
            }
        }
        // Pass 2: limits, and stop-limits triggered in pass 1.
        if let Some(ids) = self.resting.get(instrument_id).map(|v| v.clone()) {
            self.resting_pass(instrument_id, &ids, &|record| {
                let is_limit = record.intent.order_type == OrderType::Limit
                    || (record.intent.order_type == OrderType::StopLimit && record.triggered);
                if !is_limit {
                    return None;
                }
                let limit = record.intent.limit_price?.inner();
                let touched = match record.intent.side {
                    Side::Buy => lo <= limit,
                    Side::Sell => hi >= limit,
                };
                // Filled at the limit, as a maker: the bar traded through it.
                touched.then_some(Price::from_decimal(limit))
            });
        }
    }

    /// One pass over resting orders. `price_for` decides, per order, whether it
    /// is live and at what price it would fill; `None` leaves it resting.
    fn resting_pass(
        &self,
        instrument_id: &str,
        resting_ids: &[String],
        price_for: &dyn Fn(&mut PaperOrderRecord) -> Option<Price>,
    ) {
        let mut done: Vec<String> = Vec::new();
        let mut oco: Vec<(String, String)> = Vec::new();
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
            // A limit that was resting before this price — including a
            // stop-limit triggered on an earlier one — is a maker fill. A stop
            // firing now, or a stop-limit triggered by this same price, crosses.
            let was_live_limit = match record.intent.order_type {
                OrderType::Limit => true,
                OrderType::StopLimit => record.triggered,
                _ => false,
            };
            let Some(price) = price_for(&mut record) else {
                continue;
            };
            let liquidity = if was_live_limit {
                domain::fees::Liquidity::Maker
            } else {
                domain::fees::Liquidity::Taker
            };
            match self.attempt(&mut record, price, liquidity) {
                Attempt::NotMarketable => {}
                Attempt::Filled => {
                    if let Some(g) = &record.intent.oco_group {
                        oco.push((g.clone(), order_id.clone()));
                    }
                    if !record.is_open() {
                        done.push(order_id.clone());
                        drop(record);
                        self.push_terminal(order_id.clone());
                    }
                }
                Attempt::Rejected(e) | Attempt::Cancelled(e) => {
                    debug!(%order_id, error = %e, "resting paper order did not fill");
                    record.state = if matches!(e, PaperTradeError::ReduceOnlyWouldIncrease { .. }) {
                        BrokerOrderState::Cancelled
                    } else {
                        BrokerOrderState::Rejected
                    };
                    record.updated_at = self.clock.now();
                    done.push(order_id.clone());
                    drop(record);
                    self.push_terminal(order_id.clone());
                }
            }
        }
        if !done.is_empty() {
            self.remove_from_resting(instrument_id, &done);
        }
        for (group, filled) in oco {
            self.cancel_oco_siblings(&group, &filled);
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
            let mut fifo = self.lock_fifo();
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

    // ── Set U U-2.3: the order vocabulary means what it says ────────────────

    const C: AssetClass = AssetClass::CryptoSpotCex;

    fn px(v: Decimal) -> Price {
        Price::from_decimal(v)
    }

    fn mark(eng: &PaperTradingEngine, v: Decimal) {
        eng.on_mark("BTC-USD", px(v));
    }

    fn long_one(eng: &PaperTradingEngine) {
        mark(eng, dec!(50_000));
        eng.submit(C, &market("BTC-USD", Side::Buy, dec!(1)))
            .unwrap();
    }

    fn held(eng: &PaperTradingEngine) -> Decimal {
        eng.positions(C)
            .first()
            .map_or(Decimal::ZERO, |p| p.quantity)
    }

    fn typed(mut i: OrderIntent, t: OrderType) -> OrderIntent {
        i.order_type = t;
        i
    }

    fn stop_sell(qty: Decimal, stop: Decimal) -> OrderIntent {
        typed(
            market("BTC-USD", Side::Sell, qty).with_stop(px(stop)),
            OrderType::Stop,
        )
    }

    fn trailing_sell(trail: Decimal) -> OrderIntent {
        typed(
            market("BTC-USD", Side::Sell, dec!(1)).with_trail(px(trail)),
            OrderType::TrailingStop,
        )
    }

    #[test]
    fn a_stop_rests_until_its_trigger_trades_then_fills_as_a_market_order() {
        let eng = engine();
        long_one(&eng);
        let id = eng.submit(C, &stop_sell(dec!(1), dec!(49_000))).unwrap();
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
        mark(&eng, dec!(49_500));
        assert_eq!(held(&eng), dec!(1), "above the stop: nothing happens");
        mark(&eng, dec!(48_900));
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Filled
        );
        assert_eq!(held(&eng), Decimal::ZERO);
    }

    #[test]
    fn a_stop_limit_becomes_a_limit_when_triggered_not_before() {
        let eng = engine();
        long_one(&eng);
        // Before 2026-10-02 this was a plain sell limit at 48 500 — marketable at
        // 50 000, it would have sold at once.
        let mut i = typed(stop_sell(dec!(1), dec!(49_000)), OrderType::StopLimit);
        i.limit_price = Some(px(dec!(48_500)));
        let id = eng.submit(C, &i).unwrap();
        assert_eq!(held(&eng), dec!(1), "untriggered: does not sell at 50 000");
        // Gaps through both the stop and the limit: triggered, but the limit
        // protects it — it rests as a sell limit at 48 500.
        mark(&eng, dec!(48_000));
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
        assert_eq!(held(&eng), dec!(1));
        mark(&eng, dec!(48_600));
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Filled
        );
    }

    #[test]
    fn a_trailing_stop_follows_the_high_and_fires_on_the_pullback() {
        let eng = engine();
        long_one(&eng);
        let id = eng.submit(C, &trailing_sell(dec!(1_000))).unwrap();
        mark(&eng, dec!(49_200)); // 800 off the 50 000 best: holds
        mark(&eng, dec!(53_000)); // new best; stop now 52 000
        mark(&eng, dec!(52_100));
        assert_eq!(held(&eng), dec!(1));
        mark(&eng, dec!(51_900));
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Filled
        );
        assert_eq!(held(&eng), Decimal::ZERO);
    }

    #[test]
    fn reduce_only_is_capped_at_the_position_and_never_opens_one() {
        let eng = engine();
        long_one(&eng);
        let id = eng
            .submit(C, &market("BTC-USD", Side::Sell, dec!(3)).reduce_only())
            .unwrap();
        assert_eq!(held(&eng), Decimal::ZERO, "sold the one held, not three");
        assert_eq!(eng.order_status(&id).unwrap().filled_qty, dec!(1));
        // Flat: a reduce-only order has nothing to reduce.
        let err = eng
            .submit(C, &market("BTC-USD", Side::Sell, dec!(1)).reduce_only())
            .unwrap_err();
        assert!(matches!(
            err,
            PaperTradeError::ReduceOnlyWouldIncrease { .. }
        ));
        // A same-side reduce-only order would increase the position.
        long_one(&eng);
        let err = eng
            .submit(C, &market("BTC-USD", Side::Buy, dec!(1)).reduce_only())
            .unwrap_err();
        assert!(matches!(
            err,
            PaperTradeError::ReduceOnlyWouldIncrease { .. }
        ));
        assert_eq!(held(&eng), dec!(1));
    }

    #[test]
    fn post_only_refuses_to_take_and_rests_when_it_would_not() {
        let eng = engine();
        mark(&eng, dec!(50_000));
        let err = eng
            .submit(
                C,
                &limit("BTC-USD", Side::Buy, dec!(1), dec!(50_100)).post_only(),
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::PostOnlyWouldTake { .. }));
        let id = eng
            .submit(
                C,
                &limit("BTC-USD", Side::Buy, dec!(1), dec!(49_000)).post_only(),
            )
            .unwrap();
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
    }

    #[test]
    fn a_malformed_order_is_rejected_by_name() {
        let eng = engine();
        mark(&eng, dec!(50_000));
        let i = typed(market("BTC-USD", Side::Sell, dec!(1)), OrderType::Stop);
        let err = eng.submit(C, &i).unwrap_err();
        assert!(
            matches!(err, PaperTradeError::InvalidOrder(ref m) if m.contains("stop")),
            "{err}"
        );
    }

    fn bracket(eng: &PaperTradingEngine) -> (String, String) {
        let stop = eng
            .submit(
                C,
                &stop_sell(dec!(1), dec!(49_000)).reduce_only().in_oco("b1"),
            )
            .unwrap();
        let tp = eng
            .submit(
                C,
                &limit("BTC-USD", Side::Sell, dec!(1), dec!(52_000))
                    .reduce_only()
                    .in_oco("b1"),
            )
            .unwrap();
        (stop, tp)
    }

    #[test]
    fn one_cancels_other_when_either_leg_fills() {
        let eng = engine();
        long_one(&eng);
        let (stop, tp) = bracket(&eng);
        mark(&eng, dec!(52_100));
        assert_eq!(
            eng.order_status(&tp).unwrap().state,
            BrokerOrderState::Filled
        );
        assert_eq!(
            eng.order_status(&stop).unwrap().state,
            BrokerOrderState::Cancelled
        );
        assert!(eng.open_orders(Some(C)).is_empty());
        assert_eq!(held(&eng), Decimal::ZERO);
    }

    #[test]
    fn a_bar_spanning_both_bracket_legs_takes_the_stop() {
        // Pessimistic intrabar rule: the bar does not say which extreme came
        // first, so the stop is assumed to have.
        let eng = engine();
        long_one(&eng);
        let (stop, tp) = bracket(&eng);
        eng.on_bar_range("BTC-USD", px(dec!(48_000)), px(dec!(53_000)));
        let s = eng.order_status(&stop).unwrap();
        assert_eq!(s.state, BrokerOrderState::Filled);
        assert_eq!(
            eng.order_status(&tp).unwrap().state,
            BrokerOrderState::Cancelled
        );
        // Filled near its own level, not at the bar's low.
        let avg = s.avg_fill_price.unwrap().inner();
        assert!(
            avg > dec!(48_500) && avg <= dec!(49_000),
            "stop filled at {avg}"
        );
    }

    #[test]
    fn a_bar_touching_a_limit_fills_it_at_the_limit() {
        let eng = engine();
        mark(&eng, dec!(50_000));
        let id = eng
            .submit(C, &limit("BTC-USD", Side::Buy, dec!(1), dec!(49_000)))
            .unwrap();
        eng.on_bar_range("BTC-USD", px(dec!(49_500)), px(dec!(50_500)));
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
        eng.on_bar_range("BTC-USD", px(dec!(48_800)), px(dec!(50_200)));
        let s = eng.order_status(&id).unwrap();
        assert_eq!(s.state, BrokerOrderState::Filled);
        assert_eq!(s.avg_fill_price.unwrap().inner(), dec!(49_000));
    }

    #[test]
    fn a_trailing_stop_ratchets_after_the_bar_never_within_it() {
        let eng = engine();
        long_one(&eng);
        let id = eng.submit(C, &trailing_sell(dec!(1_000))).unwrap();
        // High 54 000 then: had the stop ratcheted to 53 000 inside the bar, a
        // low of 49 500 would have fired it; it ratchets only after the bar.
        eng.on_bar_range("BTC-USD", px(dec!(49_500)), px(dec!(54_000)));
        assert_eq!(eng.order_status(&id).unwrap().state, BrokerOrderState::New);
        // Next bar: the stop is 53 000 now.
        eng.on_bar_range("BTC-USD", px(dec!(52_900)), px(dec!(53_500)));
        assert_eq!(
            eng.order_status(&id).unwrap().state,
            BrokerOrderState::Filled
        );
    }

    // ── Set E #8: a poisoned account refuses orders instead of panicking ──

    fn poison(eng: &PaperTradingEngine, ac: AssetClass) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = eng.account(ac).lock();
            panic!("deliberately poisoning a paper account");
        }));
        assert!(eng.account_poisoned(ac));
    }

    #[test]
    fn a_poisoned_account_refuses_orders_and_still_answers_reads() {
        let eng = engine();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        poison(&eng, AssetClass::CryptoSpotCex);

        let err = eng
            .submit(AssetClass::CryptoSpotCex, &market("BTC-USD", Side::Buy, dec!(1)))
            .unwrap_err();
        assert!(
            matches!(err, PaperTradeError::AccountUnavailable { .. }),
            "a poisoned account must refuse with an error, got {err:?}"
        );
        // Reads do not panic.
        let _ = eng.snapshot(AssetClass::CryptoSpotCex);
        let _ = eng.positions(AssetClass::CryptoSpotCex);
        let _ = eng.daily_loss_usd(AssetClass::CryptoSpotCex);
        // Other classes are unaffected.
        assert!(!eng.account_poisoned(AssetClass::Equity));

        // A reset replaces the account and clears the poison.
        eng.reset_account(AssetClass::CryptoSpotCex);
        assert!(!eng.account_poisoned(AssetClass::CryptoSpotCex));
        eng.submit(AssetClass::CryptoSpotCex, &market("BTC-USD", Side::Buy, dec!(1)))
            .expect("a reset account trades again");
    }

    // ── Set E #5: an account's whole state round-trips ─────────────────────

    #[test]
    fn exported_state_restores_into_a_fresh_engine_exactly() {
        let a = engine();
        a.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        a.submit(AssetClass::CryptoSpotCex, &market("BTC-USD", Side::Buy, dec!(1)))
            .unwrap();
        a.on_mark("BTC-USD", Price::from_decimal(dec!(51_000)));
        a.submit(AssetClass::CryptoSpotCex, &market("BTC-USD", Side::Sell, dec!(0.5)))
            .unwrap();
        let before = a.export_account(AssetClass::CryptoSpotCex);
        assert!(!before.positions.is_empty());

        let json = serde_json::to_value(&before).unwrap();
        assert_eq!(json["asset_class"], AssetClass::CryptoSpotCex.as_str());
        let back: super::super::account::PaperAccountState = serde_json::from_value(json).unwrap();

        let b = engine();
        b.on_mark("BTC-USD", Price::from_decimal(dec!(51_000)));
        b.restore_account(AssetClass::CryptoSpotCex, back).unwrap();
        assert_eq!(b.export_account(AssetClass::CryptoSpotCex), before);
        let (sa, sb) = (
            a.snapshot(AssetClass::CryptoSpotCex),
            b.snapshot(AssetClass::CryptoSpotCex),
        );
        assert_eq!(sa.cash, sb.cash);
        assert_eq!(sa.realized_pnl, sb.realized_pnl);
        assert_eq!(sa.positions.len(), sb.positions.len());
        assert_eq!(
            a.daily_loss_usd(AssetClass::CryptoSpotCex),
            b.daily_loss_usd(AssetClass::CryptoSpotCex)
        );
    }

    #[test]
    fn a_state_for_another_class_is_refused() {
        let eng = engine();
        let state = eng.export_account(AssetClass::Equity);
        assert!(eng.restore_account(AssetClass::CryptoSpotCex, state).is_err());
    }

    #[test]
    fn touching_an_account_marks_it_dirty_once() {
        let eng = engine();
        assert!(!eng.take_dirty(AssetClass::Fx));
        let _ = eng.snapshot(AssetClass::Fx);
        assert!(eng.take_dirty(AssetClass::Fx));
        assert!(!eng.take_dirty(AssetClass::Fx));
    }

    // ── Set U U-4.7: each deployment trades in its own account ───────────────

    fn as_deployment(mut i: OrderIntent, d: Uuid) -> OrderIntent {
        i.strategy_id = Some(d.to_string());
        i
    }

    fn qty(positions: &[BrokerPosition], instrument: &str) -> Decimal {
        positions
            .iter()
            .find(|p| p.instrument_id == instrument)
            .map_or(Decimal::ZERO, |p| p.quantity)
    }

    #[test]
    fn two_deployments_on_one_class_trade_in_their_own_accounts() {
        let eng = engine();
        let ac = AssetClass::CryptoSpotCex;
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(eng.open_deployment_account(a, ac, dec!(10_000)).unwrap());
        assert!(eng.open_deployment_account(b, ac, dec!(5_000)).unwrap());
        let class_cash = eng.snapshot(ac).cash;
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(100)));
        eng.submit(ac, &as_deployment(market("BTC-USD", Side::Buy, dec!(1)), a)).unwrap();
        eng.submit(ac, &as_deployment(market("BTC-USD", Side::Buy, dec!(2)), b)).unwrap();

        let (sa, sb) = (a.to_string(), b.to_string());
        assert_eq!(qty(&eng.positions_for(ac, Some(&sa)), "BTC-USD"), dec!(1));
        assert_eq!(qty(&eng.positions_for(ac, Some(&sb)), "BTC-USD"), dec!(2));
        assert_eq!(qty(&eng.positions(ac), "BTC-USD"), Decimal::ZERO, "the class account traded nothing");
        assert_eq!(eng.snapshot(ac).cash, class_cash);
        assert!(eng.snapshot_for(ac, Some(&sa)).cash < dec!(10_000));
        assert!(eng.snapshot_for(ac, Some(&sb)).cash < dec!(5_000));
        assert!(
            dec!(5_000) - eng.snapshot_for(ac, Some(&sb)).cash
                > dec!(10_000) - eng.snapshot_for(ac, Some(&sa)).cash,
            "B bought twice as much from its own cash"
        );
    }

    #[test]
    fn an_order_naming_no_opened_account_trades_in_the_class_account() {
        let eng = engine();
        let ac = AssetClass::CryptoSpotCex;
        eng.on_mark("ETH-USD", Price::from_decimal(dec!(50)));
        // A deployment that never opened an account, and a manual order.
        eng.submit(ac, &as_deployment(market("ETH-USD", Side::Buy, dec!(1)), Uuid::new_v4()))
            .unwrap();
        eng.submit(ac, &market("ETH-USD", Side::Buy, dec!(1))).unwrap();
        assert_eq!(qty(&eng.positions(ac), "ETH-USD"), dec!(2));
        // An account opened on another class does not capture this class's orders.
        let fx = Uuid::new_v4();
        eng.open_deployment_account(fx, AssetClass::Fx, dec!(1_000)).unwrap();
        eng.submit(ac, &as_deployment(market("ETH-USD", Side::Buy, dec!(1)), fx)).unwrap();
        assert_eq!(qty(&eng.positions(ac), "ETH-USD"), dec!(3));
    }

    #[test]
    fn reduce_only_reads_the_deployments_own_position() {
        let eng = engine();
        let ac = AssetClass::CryptoSpotCex;
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        eng.open_deployment_account(a, ac, dec!(10_000)).unwrap();
        eng.open_deployment_account(b, ac, dec!(10_000)).unwrap();
        eng.on_mark("SOL-USD", Price::from_decimal(dec!(20)));
        eng.submit(ac, &as_deployment(market("SOL-USD", Side::Buy, dec!(3)), a)).unwrap();
        // B is flat: its reduce-only sell has nothing to reduce, whatever A holds.
        let mut sell_b = as_deployment(market("SOL-USD", Side::Sell, dec!(1)), b);
        sell_b.reduce_only = true;
        assert!(eng.submit(ac, &sell_b).is_err());
        let mut sell_a = as_deployment(market("SOL-USD", Side::Sell, dec!(3)), a);
        sell_a.reduce_only = true;
        eng.submit(ac, &sell_a).unwrap();
        assert_eq!(qty(&eng.positions_for(ac, Some(&a.to_string())), "SOL-USD"), Decimal::ZERO);
    }

    #[test]
    fn a_settlement_reaches_a_deployments_position() {
        let eng = engine();
        let ac = AssetClass::CryptoSpotCex;
        let a = Uuid::new_v4();
        eng.open_deployment_account(a, ac, dec!(10_000)).unwrap();
        eng.on_mark("XRP-USD", Price::from_decimal(dec!(10)));
        eng.submit(ac, &as_deployment(market("XRP-USD", Side::Buy, dec!(5)), a)).unwrap();
        eng.settle_at_price(ac, "XRP-USD", dec!(12)).unwrap();
        assert_eq!(qty(&eng.positions_for(ac, Some(&a.to_string())), "XRP-USD"), Decimal::ZERO);
        // With no holder anywhere, the class account answers with the error.
        assert!(eng.settle_at_price(ac, "XRP-USD", dec!(12)).is_err());
    }

    #[test]
    fn a_deployment_account_round_trips_and_is_never_reseeded() {
        let eng = engine();
        let ac = AssetClass::CryptoSpotCex;
        let a = Uuid::new_v4();
        eng.open_deployment_account(a, ac, dec!(10_000)).unwrap();
        eng.on_mark("BTC-USD", Price::from_decimal(dec!(100)));
        eng.submit(ac, &as_deployment(market("BTC-USD", Side::Buy, dec!(1)), a)).unwrap();
        assert!(eng.take_deployment_dirty(a));
        assert!(!eng.take_deployment_dirty(a));
        let (class, state) = eng.export_deployment_account(a).unwrap();
        assert_eq!(class, ac);

        // A restart: a fresh engine restores it, and opening it again keeps it.
        let fresh = engine();
        fresh.restore_deployment_account(a, ac, state).unwrap();
        assert!(!fresh.open_deployment_account(a, ac, dec!(99)).unwrap());
        let sa = a.to_string();
        assert_eq!(qty(&fresh.positions_for(ac, Some(&sa)), "BTC-USD"), dec!(1));
        assert_eq!(
            fresh.snapshot_for(ac, Some(&sa)).cash,
            eng.snapshot_for(ac, Some(&sa)).cash
        );
        assert_eq!(fresh.deployment_accounts(), vec![(a, ac)]);
    }
}

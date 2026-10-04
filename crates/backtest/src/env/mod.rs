//! The backtest environment (Set U U-3.1, ADR-0039): the strategy engine's ports
//! implemented over history.
//!
//! - [`MergedBars`] — every input's bars in the order they closed.
//! - [`SimOrders`] — the engine's venue port over the [`SimulatedVenue`] (the
//!   paper venue's core on a simulated clock) behind the production risk gate
//!   ([`SimulatedRisk`]).
//! - [`TradeBook`] — round trips rebuilt from the fills the venue reported, in
//!   the shape Set J judges.
//!
//! Nothing here decides anything. Decisions are the engine's, driven through
//! [`strategy_engine::cycle`] exactly as paper drives them.

pub mod universe;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use domain::money::{Price, Size};
use domain::order::{OrderIntent, OrderType, Side};
use execution::broker::BrokerOrderState;
use execution::sim_risk::{Refusal, SimulatedRisk};
use execution::sim_venue::{FillPolicy, SimBar, SimulatedVenue};
use execution::Venue;
use rust_decimal::Decimal;
use strategy_engine::env::{MarketData, MarketEvent, VenuePort};
use strategy_engine::planner::{OrderAction, OrderRole, PlannedOrder, PlannedStyle};
use strategy_engine::{
    Bar, Event, Fill, InstrumentSpec, OrderClosed, OrderClosedReason, PortfolioView, WorkingOrder,
};
use uuid::Uuid;

use crate::run::result::{GateRejectionRecord, Side as TradeSide, Trade};

/// Unix nanoseconds → UTC.
#[must_use]
pub fn at(ns: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(ns)
}

// ── market data ──────────────────────────────────────────────────────────────

/// Every input's bars merged into one stream ordered by close.
///
/// At equal close instants the other inputs come before the primary: a BTC
/// daily bar that closes at the same instant as an ETH hourly decision bar has
/// closed *by* that decision, so the decision sees it. A bar closing one
/// nanosecond later does not exist yet — which is the whole point-in-time rule
/// for multi-input strategies (U-3.2).
pub struct MergedBars {
    events: Vec<MarketEvent>,
    next: usize,
}

impl MergedBars {
    /// `inputs[i]` are input `i`'s bars. Bars closing at or before
    /// `window_start_ns` are warm-up.
    #[must_use]
    pub fn new(inputs: Vec<Vec<Bar>>, window_start_ns: i64) -> Self {
        let mut events: Vec<MarketEvent> = inputs
            .into_iter()
            .enumerate()
            .flat_map(|(input, bars)| {
                bars.into_iter().map(move |bar| MarketEvent {
                    input,
                    bar,
                    warmup: bar.close_ns <= window_start_ns,
                })
            })
            .collect();
        events.sort_by_key(|e| {
            (
                e.bar.close_ns,
                e.input == strategy_engine::plan::PRIMARY,
                e.input,
            )
        });
        Self { events, next: 0 }
    }
}

impl MarketData for MergedBars {
    fn next_bar(&mut self) -> Option<MarketEvent> {
        let e = self.events.get(self.next).copied();
        self.next += 1;
        e
    }
}

// ── venues a backtest can drive ──────────────────────────────────────────────

/// A venue the backtest loop steps bar by bar. The engine sees only its
/// [`VenuePort`] face; the loop also tells it when a primary bar opens and when
/// the decision on its close has been made.
pub trait BarVenue: VenuePort {
    /// The primary bar's range is about to be traded. Returns what happened to
    /// this strategy's orders, as engine events, before the close is decided.
    fn on_bar(&mut self, bar: &SimBar) -> Vec<Event>;
    /// The decision on `bar`'s close has been made.
    fn on_close_decided(&mut self, _bar: &SimBar) {}
}

/// A venue that records what the strategy *would* send and never executes it
/// (U-3.5's intent-only preview). The account stays flat at its opening
/// equity, so this answers "when would it act?" — not "what would it hold?".
pub struct IntentOnly {
    spec: InstrumentSpec,
    equity: Decimal,
    mark: Option<Decimal>,
    actions: Vec<(i64, OrderAction)>,
}

impl IntentOnly {
    #[must_use]
    pub fn new(spec: InstrumentSpec, equity: Decimal) -> Self {
        Self {
            spec,
            equity,
            mark: None,
            actions: Vec::new(),
        }
    }

    /// Every action the strategy asked for, with its instant.
    #[must_use]
    pub fn actions(&self) -> &[(i64, OrderAction)] {
        &self.actions
    }
}

impl VenuePort for IntentOnly {
    fn view(&self) -> PortfolioView {
        PortfolioView {
            equity: self.equity,
            cash: self.equity,
            mark: self.mark,
            ..PortfolioView::default()
        }
    }
    fn spec(&self) -> InstrumentSpec {
        self.spec.clone()
    }
    fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> Vec<Event> {
        self.actions
            .extend(actions.iter().cloned().map(|a| (at_ns, a)));
        Vec::new()
    }
}

impl BarVenue for IntentOnly {
    fn on_bar(&mut self, bar: &SimBar) -> Vec<Event> {
        self.mark = Some(bar.close);
        Vec::new()
    }
}

// ── the venue port ───────────────────────────────────────────────────────────

/// One order the planner placed, as this environment tracks it.
#[derive(Clone, Debug)]
struct Tracked {
    seq: u64,
    key: Uuid,
    side: Side,
    qty: Decimal,
    reduce_only: bool,
    protective: bool,
    filled: Decimal,
    /// Σ qty × price over fills so far.
    notional: Decimal,
    commission: Decimal,
    closed: bool,
}

/// A fill as the trade book needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct FillRecord {
    pub tag: String,
    pub side: Side,
    pub qty: Decimal,
    pub price: Decimal,
    pub fee: Decimal,
    pub at_ns: i64,
}

/// A round trip and the trading instance that opened it (U-6.5).
#[derive(Clone, Debug, PartialEq)]
pub struct AttributedTrade {
    pub instance: String,
    pub trade: Trade,
}

/// One instrument's book inside the account: its orders, its position's own
/// clock and anchor, and its round trips. A single-instrument run has one; a
/// portfolio run has one per instrument it ever traded.
struct Sleeve {
    spec: InstrumentSpec,
    /// The trading instance currently deciding for this instrument, if any.
    instance: String,
    /// Idempotency keys are `v5(key_prefix/tag)`: a portfolio's instances may
    /// reuse planner tags, so each instance gets its own prefix.
    key_prefix: String,
    orders: BTreeMap<String, Tracked>,
    mark: Option<Decimal>,
    bars_in_position: u32,
    high_water: Option<Decimal>,
    last_fill_price: Option<Decimal>,
    book: TradeBook,
    /// The instance that opened the position now held.
    opened_by: String,
    attributed: Vec<AttributedTrade>,
    /// This instance's share of the deployment's capital. `None`: the whole
    /// account, as a single-instrument run sees it.
    capital: Option<Decimal>,
    /// What the selection said about this member at its latest rebalance
    /// (U-6.6). `None` outside a portfolio run.
    selection: Option<strategy_engine::SelectionContext>,
}

impl Sleeve {
    fn new(
        spec: InstrumentSpec,
        instance: String,
        key_prefix: String,
        capital: Option<Decimal>,
    ) -> Self {
        Self {
            book: TradeBook::new(spec.instrument.clone()),
            spec,
            opened_by: instance.clone(),
            instance,
            key_prefix,
            orders: BTreeMap::new(),
            mark: None,
            bars_in_position: 0,
            high_water: None,
            last_fill_price: None,
            attributed: Vec::new(),
            capital,
            selection: None,
        }
    }

    fn book_fill(&mut self, record: &FillRecord) {
        if self.book.position().is_zero() {
            self.opened_by.clone_from(&self.instance);
        }
        let before = self.book.closed_len();
        self.book.on_fill(record);
        for trade in self.book.closed_since(before) {
            self.attributed.push(AttributedTrade {
                instance: self.opened_by.clone(),
                trade,
            });
        }
        if self.book.closed_len() > before && !self.book.position().is_zero() {
            // A flip: the new position is this instance's.
            self.opened_by.clone_from(&self.instance);
        }
    }
}

/// The engine's venue port over the simulated venue and the risk gate.
///
/// One account. A single-instrument run binds one instrument and uses the
/// [`VenuePort`] impl directly; a portfolio run (U-6.5) opens a sleeve per
/// instrument and hands each instance a [`SleevePort`]. Both go through the same
/// order bookkeeping below — there is one place orders become venue intents and
/// venue reports become engine events.
pub struct SimOrders {
    venue: SimulatedVenue,
    risk: SimulatedRisk,
    asset_class: AssetClass,
    strategy_id: String,
    sleeves: BTreeMap<String, Sleeve>,
    /// The instrument the single-instrument [`VenuePort`] impl acts for.
    primary: String,
    seq: u64,
    fills: Vec<FillRecord>,
    /// Gate refusals by the venue's idempotency key → planner tag.
    tags_by_key: BTreeMap<Uuid, String>,
    equity: Vec<(DateTime<Utc>, f64)>,
    exposure: Vec<(DateTime<Utc>, f64)>,
}

/// What a simulated run accumulated.
pub struct SimResults {
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub net_exposure: Vec<(DateTime<Utc>, f64)>,
    pub trades: Vec<Trade>,
    pub gate_rejections: Vec<GateRejectionRecord>,
}

/// What a portfolio run accumulated: every round trip with the instance that
/// opened it.
pub struct PortfolioResults {
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub net_exposure: Vec<(DateTime<Utc>, f64)>,
    pub trades: Vec<AttributedTrade>,
    pub gate_rejections: Vec<GateRejectionRecord>,
    /// Final account equity.
    pub final_equity: Decimal,
}

impl SimOrders {
    #[must_use]
    pub fn new(
        venue: SimulatedVenue,
        risk: SimulatedRisk,
        asset_class: AssetClass,
        spec: InstrumentSpec,
        strategy_id: impl Into<String>,
    ) -> Self {
        let strategy_id = strategy_id.into();
        let primary = spec.instrument.clone();
        let sleeve = Sleeve::new(spec, strategy_id.clone(), strategy_id.clone(), None);
        Self {
            venue,
            risk,
            asset_class,
            strategy_id,
            sleeves: BTreeMap::from([(primary.clone(), sleeve)]),
            primary,
            seq: 0,
            fills: Vec::new(),
            tags_by_key: BTreeMap::new(),
            equity: Vec::new(),
            exposure: Vec::new(),
        }
    }

    /// An account with no instrument bound yet: a portfolio run opens sleeves
    /// as instances start.
    #[must_use]
    pub fn portfolio(
        venue: SimulatedVenue,
        risk: SimulatedRisk,
        asset_class: AssetClass,
        strategy_id: impl Into<String>,
    ) -> Self {
        Self {
            venue,
            risk,
            asset_class,
            strategy_id: strategy_id.into(),
            sleeves: BTreeMap::new(),
            primary: String::new(),
            seq: 0,
            fills: Vec::new(),
            tags_by_key: BTreeMap::new(),
            equity: Vec::new(),
            exposure: Vec::new(),
        }
    }

    /// Everything the run accumulated.
    #[must_use]
    pub fn into_results(mut self) -> SimResults {
        let gate_rejections = self.gate_rejections();
        let trades = self
            .sleeves
            .remove(&self.primary)
            .map(|s| s.book.trades())
            .unwrap_or_default();
        SimResults {
            equity: self.equity,
            net_exposure: self.exposure,
            trades,
            gate_rejections,
        }
    }

    /// Everything a portfolio run accumulated.
    #[must_use]
    pub fn into_portfolio_results(self) -> PortfolioResults {
        let gate_rejections = self.gate_rejections();
        let final_equity = self.equity();
        let mut trades: Vec<AttributedTrade> = self
            .sleeves
            .into_values()
            .flat_map(|s| s.attributed)
            .collect();
        trades.sort_by(|a, b| {
            (a.trade.exit_time, &a.trade.symbol).cmp(&(b.trade.exit_time, &b.trade.symbol))
        });
        PortfolioResults {
            equity: self.equity,
            net_exposure: self.exposure,
            trades,
            gate_rejections,
            final_equity,
        }
    }

    #[must_use]
    pub const fn venue(&self) -> &SimulatedVenue {
        &self.venue
    }

    /// Every fill so far, in the order the venue reported them.
    #[must_use]
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }

    /// The position in the bound instrument.
    #[must_use]
    pub fn position(&self) -> Decimal {
        self.position_of(&self.primary)
    }

    /// The position in `instrument`.
    #[must_use]
    pub fn position_of(&self, instrument: &str) -> Decimal {
        self.venue
            .positions(self.asset_class)
            .iter()
            .find(|p| p.instrument_id == instrument)
            .map_or(Decimal::ZERO, |p| p.quantity)
    }

    /// Account equity at the latest marks.
    #[must_use]
    pub fn equity(&self) -> Decimal {
        self.venue.snapshot(self.asset_class).equity
    }

    /// Every order the risk gate refused, with the planner's tag.
    #[must_use]
    pub fn gate_rejections(&self) -> Vec<GateRejectionRecord> {
        self.risk
            .rejections()
            .into_iter()
            .map(|r| GateRejectionRecord {
                at: r.at,
                instrument: r.instrument_id,
                tag: self.tags_by_key.get(&r.key).cloned().unwrap_or_default(),
                check: rejection_check(&r.rejection),
                detail: r.rejection.to_string(),
            })
            .collect()
    }

    // ── portfolio sleeves ────────────────────────────────────────────────────

    /// Start (or restart) `instance` trading `spec.instrument` with `capital`.
    /// A position the instrument still holds — kept under `hold` — is the new
    /// instance's to manage.
    pub fn open_sleeve(
        &mut self,
        spec: InstrumentSpec,
        instance: impl Into<String>,
        capital: Decimal,
    ) {
        let instance = instance.into();
        let prefix = format!("{}/{instance}", self.strategy_id);
        if let Some(s) = self.sleeves.get_mut(&spec.instrument) {
            s.instance = instance;
            s.key_prefix = prefix;
            s.capital = Some(capital);
            s.spec = spec;
        } else {
            let key = spec.instrument.clone();
            self.sleeves
                .insert(key, Sleeve::new(spec, instance, prefix, Some(capital)));
        }
    }

    /// Run one bar of any instrument through the venue; what happened to that
    /// instrument's orders comes back as engine events.
    pub fn on_bar_of(&mut self, bar: &SimBar) -> Vec<Event> {
        self.bar_on(bar)
    }

    /// A sleeve's share of the deployment's capital changed.
    pub fn set_capital(&mut self, instrument: &str, capital: Decimal) {
        if let Some(s) = self.sleeves.get_mut(instrument) {
            s.capital = Some(capital);
        }
    }

    /// What the selection said about `instrument`'s member (U-6.6).
    pub fn set_selection(&mut self, instrument: &str, selection: strategy_engine::SelectionContext) {
        if let Some(s) = self.sleeves.get_mut(instrument) {
            s.selection = Some(selection);
        }
    }

    /// The port an instance on `instrument` decides through.
    ///
    /// # Panics
    /// No sleeve is open for `instrument`.
    pub fn port<'a>(&'a mut self, instrument: &'a str) -> SleevePort<'a> {
        assert!(
            self.sleeves.contains_key(instrument),
            "no sleeve for {instrument}"
        );
        SleevePort {
            orders: self,
            instrument,
        }
    }

    /// Flat, with no working order: an instance here has nothing left to do.
    #[must_use]
    pub fn is_idle(&self, instrument: &str) -> bool {
        self.position_of(instrument).is_zero()
            && self
                .sleeves
                .get(instrument)
                .is_none_or(|s| s.orders.values().all(|t| t.closed))
    }

    /// `close` for a leaver: cancel its working orders and flatten it with a
    /// reduce-only market order, through the risk gate like any other order.
    pub fn flatten(&mut self, instrument: &str, at_ns: i64) -> Vec<Event> {
        let Some(s) = self.sleeves.get(instrument) else {
            return Vec::new();
        };
        let mut actions: Vec<OrderAction> = s
            .orders
            .iter()
            .filter(|(_, t)| !t.closed)
            .map(|(tag, _)| OrderAction::Cancel { tag: tag.clone() })
            .collect();
        let pos = self.position_of(instrument);
        if !pos.is_zero() {
            actions.push(OrderAction::Submit(PlannedOrder {
                tag: format!("allocator:close:{at_ns}"),
                side: if pos.is_sign_positive() {
                    Side::Sell
                } else {
                    Side::Buy
                },
                qty: pos.abs(),
                style: PlannedStyle::Market,
                tif: domain::order::TimeInForce::Gtc,
                reduce_only: true,
                post_only: false,
                oco_group: None,
                role: OrderRole::Exit,
            }));
        }
        self.act_on(instrument, &actions, at_ns)
    }

    /// Equity and net exposure across every sleeve, sampled once per instant
    /// after every decision on it.
    pub fn sample(&mut self, at_ns: i64) {
        use rust_decimal::prelude::ToPrimitive;
        let eq = self.equity();
        let gross: Decimal = self
            .sleeves
            .iter()
            .map(|(m, s)| self.position_of(m) * s.mark.unwrap_or_default())
            .sum();
        self.equity.push((at(at_ns), eq.to_f64().unwrap_or(0.0)));
        let expo = if eq.is_zero() {
            0.0
        } else {
            (gross / eq).to_f64().unwrap_or(0.0)
        };
        self.exposure.push((at(at_ns), expo));
    }

    // ── one instrument's bookkeeping, shared by both shapes ──────────────────

    /// What the venue did to `instrument`'s orders since the last look, as
    /// engine events: fills first, in submission order, then orders the venue
    /// cancelled or refused. Fills are booked into the trade book as they are
    /// found.
    fn collect(&mut self, instrument: &str, at_ns: i64) -> Vec<Event> {
        let Some(sleeve) = self.sleeves.get(instrument) else {
            return Vec::new();
        };
        let mut live: Vec<(String, Tracked)> = sleeve
            .orders
            .iter()
            .filter(|(_, t)| !t.closed)
            .map(|(k, t)| (k.clone(), t.clone()))
            .collect();
        live.sort_by_key(|(_, t)| t.seq);
        let mut events = Vec::new();
        for (tag, mut t) in live {
            let Some(status) = self.venue.order(t.key) else {
                continue; // still on its way to the venue
            };
            let sleeve = self.sleeves.get_mut(instrument).expect("checked above");
            let delta = status.filled_qty - t.filled;
            if delta > Decimal::ZERO {
                let total = status
                    .avg_fill_price
                    .map_or(Decimal::ZERO, |p| p.inner() * status.filled_qty);
                let price = (total - t.notional) / delta;
                let commission = status.commission.unwrap_or_default();
                let fee = commission - t.commission;
                t.filled = status.filled_qty;
                t.notional = total;
                t.commission = commission;
                sleeve.last_fill_price = Some(price);
                let record = FillRecord {
                    tag: tag.clone(),
                    side: t.side,
                    qty: delta,
                    price,
                    fee,
                    at_ns,
                };
                sleeve.book_fill(&record);
                self.fills.push(record);
                events.push(Event::Fill(Fill {
                    tag: tag.clone(),
                    side: t.side,
                    qty: delta,
                    price,
                    at_ns,
                }));
            }
            match status.state {
                BrokerOrderState::Filled => t.closed = true,
                BrokerOrderState::Cancelled | BrokerOrderState::Rejected => {
                    t.closed = true;
                    events.push(Event::OrderClosed(OrderClosed {
                        tag: tag.clone(),
                        reason: if status.state == BrokerOrderState::Cancelled {
                            OrderClosedReason::Cancelled
                        } else {
                            OrderClosedReason::Rejected
                        },
                        at_ns,
                    }));
                }
                _ => {}
            }
            sleeve.orders.insert(tag, t);
        }
        events
    }

    fn intent(&self, sleeve: &Sleeve, o: &PlannedOrder) -> OrderIntent {
        let (order_type, limit, stop, trail) = match &o.style {
            PlannedStyle::Market => (OrderType::Market, None, None, None),
            PlannedStyle::Limit { price } => (OrderType::Limit, Some(*price), None, None),
            PlannedStyle::Stop { trigger } => (OrderType::Stop, None, Some(*trigger), None),
            PlannedStyle::StopLimit { trigger, price } => {
                (OrderType::StopLimit, Some(*price), Some(*trigger), None)
            }
            PlannedStyle::TrailingStop { trail } => {
                (OrderType::TrailingStop, None, None, Some(*trail))
            }
        };
        let mut i = OrderIntent::new(
            sleeve.spec.instrument.clone(),
            o.side,
            order_type,
            Size::from_decimal(o.qty),
            limit.map(Price::from_decimal),
            Some(self.strategy_id.clone()),
        );
        // Deterministic: the same run places the same keys.
        i.idempotency_key = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}/{}", sleeve.key_prefix, o.tag).as_bytes(),
        );
        i.time_in_force = o.tif;
        i.stop_price = stop.map(Price::from_decimal);
        i.trail = trail.map(Price::from_decimal);
        i.reduce_only = o.reduce_only;
        i.post_only = o.post_only;
        i.oco_group.clone_from(&o.oco_group);
        i
    }

    /// Run one bar of `bar.instrument_id` through the venue. Fills are stamped
    /// at the bar's open: they happen inside the bar, before its close is
    /// decided.
    fn bar_on(&mut self, bar: &SimBar) -> Vec<Event> {
        let _report = self.venue.on_bar(bar);
        let instrument = bar.instrument_id.clone();
        let events = self.collect(
            &instrument,
            bar.open_time.timestamp_nanos_opt().unwrap_or(0),
        );
        let pos = self.position_of(&instrument);
        let Some(s) = self.sleeves.get_mut(&instrument) else {
            return events;
        };
        s.book.on_bar(bar.high, bar.low);
        // The position's own clock and anchor, measured from the bars.
        s.mark = Some(bar.close);
        if pos.is_zero() {
            s.bars_in_position = 0;
            s.high_water = None;
        } else {
            s.bars_in_position += 1;
            let favourable = if pos.is_sign_positive() {
                bar.high
            } else {
                bar.low
            };
            s.high_water = Some(s.high_water.map_or(favourable, |h| {
                if pos.is_sign_positive() {
                    h.max(favourable)
                } else {
                    h.min(favourable)
                }
            }));
        }
        events
    }

    fn view_of(&self, instrument: &str) -> PortfolioView {
        let snap = self.venue.snapshot(self.asset_class);
        let pos = snap
            .positions
            .iter()
            .find(|p| p.instrument_id == instrument);
        let unrealized: Decimal = snap.positions.iter().map(|p| p.unrealized_pnl).sum();
        let daily_loss = self.venue.engine().daily_loss_usd(self.asset_class);
        let sleeve = self.sleeves.get(instrument);
        let open_orders = sleeve
            .map(|s| {
                s.orders
                    .iter()
                    .filter(|(_, t)| !t.closed)
                    .map(|(tag, t)| WorkingOrder {
                        tag: tag.clone(),
                        side: t.side,
                        remaining: t.qty - t.filled,
                        reduce_only: t.reduce_only,
                        protective: t.protective,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let position = pos.map_or(Decimal::ZERO, |p| p.quantity);
        let mark = sleeve.and_then(|s| s.mark);
        // A sleeve sizes against its share of the deployment's capital; the
        // single-instrument run against the whole account.
        let (equity, cash) = match sleeve.and_then(|s| s.capital) {
            Some(c) => (c, c - position * mark.unwrap_or_default()),
            None => (snap.equity, snap.cash),
        };
        PortfolioView {
            position,
            avg_entry_price: pos.map(|p| p.average_entry_price),
            bars_in_position: sleeve.map_or(0, |s| s.bars_in_position),
            high_water: sleeve.and_then(|s| s.high_water),
            equity,
            cash,
            realized_pnl_today: -daily_loss - unrealized,
            mark,
            open_orders,
            last_fill_price: sleeve.and_then(|s| s.last_fill_price),
            selection: sleeve.and_then(|s| s.selection.clone()),
        }
    }

    fn act_on(&mut self, instrument: &str, actions: &[OrderAction], at_ns: i64) -> Vec<Event> {
        let mut events = Vec::new();
        for action in actions {
            match action {
                OrderAction::Submit(o) => {
                    let Some(sleeve) = self.sleeves.get(instrument) else {
                        continue;
                    };
                    let intent = self.intent(sleeve, o);
                    self.seq += 1;
                    self.tags_by_key
                        .insert(intent.idempotency_key, o.tag.clone());
                    let mut t = Tracked {
                        seq: self.seq,
                        key: intent.idempotency_key,
                        side: o.side,
                        qty: o.qty,
                        reduce_only: o.reduce_only,
                        protective: o.role == OrderRole::Protective,
                        filled: Decimal::ZERO,
                        notional: Decimal::ZERO,
                        commission: Decimal::ZERO,
                        closed: false,
                    };
                    if let Err(refusal) = self.risk.submit(&self.venue, self.asset_class, &intent) {
                        if let Refusal::Venue(e) = &refusal {
                            tracing::debug!(tag = %o.tag, error = %e, "simulated venue refused an order");
                        }
                        t.closed = true;
                        events.push(Event::OrderClosed(OrderClosed {
                            tag: o.tag.clone(),
                            reason: OrderClosedReason::Rejected,
                            at_ns,
                        }));
                    }
                    if let Some(s) = self.sleeves.get_mut(instrument) {
                        s.orders.insert(o.tag.clone(), t);
                    }
                }
                OrderAction::Cancel { tag } => {
                    let Some(t) = self
                        .sleeves
                        .get_mut(instrument)
                        .and_then(|s| s.orders.get_mut(tag))
                    else {
                        continue;
                    };
                    if t.closed {
                        continue;
                    }
                    let _ = self.venue.cancel(t.key);
                    t.closed = true;
                    events.push(Event::OrderClosed(OrderClosed {
                        tag: tag.clone(),
                        reason: OrderClosedReason::Cancelled,
                        at_ns,
                    }));
                }
            }
        }
        // Under `current_close` an order executes the moment it is submitted;
        // the engine hears of the fill now, stamped at the decision instant.
        if self.venue.policy() == FillPolicy::CurrentClose {
            events.extend(self.collect(instrument, at_ns));
        }
        events
    }
}

impl BarVenue for SimOrders {
    /// Run one bar through the venue, for whichever instrument it is.
    fn on_bar(&mut self, bar: &SimBar) -> Vec<Event> {
        self.bar_on(bar)
    }

    /// Equity and exposure, sampled after the close is decided — after any
    /// order the decision filled at once under `current_close`.
    fn on_close_decided(&mut self, bar: &SimBar) {
        use rust_decimal::prelude::ToPrimitive;
        let eq = self.equity();
        self.equity
            .push((bar.close_time, eq.to_f64().unwrap_or(0.0)));
        let expo = if eq.is_zero() {
            0.0
        } else {
            (self.position() * bar.close / eq).to_f64().unwrap_or(0.0)
        };
        self.exposure.push((bar.close_time, expo));
    }
}

impl strategy_engine::VenuePort for SimOrders {
    fn view(&self) -> PortfolioView {
        self.view_of(&self.primary)
    }

    fn spec(&self) -> InstrumentSpec {
        self.sleeves
            .get(&self.primary)
            .map(|s| s.spec.clone())
            .expect("a single-instrument run binds its instrument")
    }

    fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> Vec<Event> {
        let primary = self.primary.clone();
        self.act_on(&primary, actions, at_ns)
    }
}

/// One portfolio instance's face of the shared account (U-6.5).
pub struct SleevePort<'a> {
    orders: &'a mut SimOrders,
    instrument: &'a str,
}

impl strategy_engine::VenuePort for SleevePort<'_> {
    fn view(&self) -> PortfolioView {
        self.orders.view_of(self.instrument)
    }

    fn spec(&self) -> InstrumentSpec {
        self.orders.sleeves[self.instrument].spec.clone()
    }

    fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> Vec<Event> {
        self.orders.act_on(self.instrument, actions, at_ns)
    }
}

/// The variant name of a refusal: `PositionLimitExceeded`, `KillSwitchActive`…
fn rejection_check(r: &domain::RiskRejection) -> String {
    match serde_json::to_value(r) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(serde_json::Value::Object(m)) => m.keys().next().cloned().unwrap_or_default(),
        _ => format!("{r:?}"),
    }
}

// ── round trips ──────────────────────────────────────────────────────────────

struct OpenTrade {
    long: bool,
    entry_ns: i64,
    entry_qty: Decimal,
    entry_value: Decimal,
    exit_qty: Decimal,
    exit_value: Decimal,
    realized: Decimal,
    fees: Decimal,
    high: Decimal,
    low: Decimal,
}

/// Round trips from flat to flat, rebuilt from fills.
pub struct TradeBook {
    symbol: String,
    position: Decimal,
    open: Option<OpenTrade>,
    closed: Vec<Trade>,
}

impl TradeBook {
    #[must_use]
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            position: Decimal::ZERO,
            open: None,
            closed: Vec::new(),
        }
    }

    /// Every closed round trip.
    #[must_use]
    pub fn trades(self) -> Vec<Trade> {
        self.closed
    }

    /// The position the fills so far add up to.
    #[must_use]
    pub const fn position(&self) -> Decimal {
        self.position
    }

    /// How many round trips have closed.
    #[must_use]
    pub fn closed_len(&self) -> usize {
        self.closed.len()
    }

    /// The round trips closed after the first `n`.
    #[must_use]
    pub fn closed_since(&self, n: usize) -> Vec<Trade> {
        self.closed
            .get(n..)
            .map(<[Trade]>::to_vec)
            .unwrap_or_default()
    }

    /// Extend the open trade's excursion with a bar's range.
    pub fn on_bar(&mut self, high: Decimal, low: Decimal) {
        if let Some(t) = &mut self.open {
            t.high = t.high.max(high);
            t.low = t.low.min(low);
        }
    }

    pub fn on_fill(&mut self, f: &FillRecord) {
        if f.qty <= Decimal::ZERO {
            return;
        }
        let signed = if f.side == Side::Buy { f.qty } else { -f.qty };
        if self.open.is_none() {
            self.open_with(f.side == Side::Buy, f.qty, f.price, f.fee, f.at_ns);
            self.position += signed;
            return;
        }
        let long = self.open.as_ref().is_some_and(|t| t.long);
        let adds = (f.side == Side::Buy) == long;
        if adds {
            if let Some(t) = &mut self.open {
                t.entry_qty += f.qty;
                t.entry_value += f.qty * f.price;
                t.fees += f.fee;
            }
            self.position += signed;
            return;
        }
        let reduce = f.qty.min(self.position.abs());
        let fee_reduce = if f.qty.is_zero() {
            Decimal::ZERO
        } else {
            f.fee * reduce / f.qty
        };
        if let Some(t) = &mut self.open {
            let avg = t.entry_value / t.entry_qty;
            let dir = if t.long { Decimal::ONE } else { -Decimal::ONE };
            t.realized += (f.price - avg) * reduce * dir;
            t.exit_qty += reduce;
            t.exit_value += reduce * f.price;
            t.fees += fee_reduce;
        }
        self.position += signed;
        if self.position.is_zero() || self.position.is_sign_positive() != long {
            self.close(f.at_ns);
            let rest = f.qty - reduce;
            if rest > Decimal::ZERO {
                self.open_with(
                    f.side == Side::Buy,
                    rest,
                    f.price,
                    f.fee - fee_reduce,
                    f.at_ns,
                );
            }
        }
    }

    fn open_with(&mut self, long: bool, qty: Decimal, price: Decimal, fee: Decimal, at_ns: i64) {
        self.open = Some(OpenTrade {
            long,
            entry_ns: at_ns,
            entry_qty: qty,
            entry_value: qty * price,
            exit_qty: Decimal::ZERO,
            exit_value: Decimal::ZERO,
            realized: Decimal::ZERO,
            fees: fee,
            high: price,
            low: price,
        });
    }

    fn close(&mut self, at_ns: i64) {
        let Some(t) = self.open.take() else { return };
        use rust_decimal::prelude::ToPrimitive;
        let entry = t.entry_value / t.entry_qty;
        let exit = if t.exit_qty.is_zero() {
            entry
        } else {
            t.exit_value / t.exit_qty
        };
        let excursion = |p: Decimal| ((p - entry) / entry).to_f64().unwrap_or(0.0);
        let (mae, mfe) = if t.long {
            (excursion(t.low).min(0.0), excursion(t.high).max(0.0))
        } else {
            ((-excursion(t.high)).min(0.0), (-excursion(t.low)).max(0.0))
        };
        self.closed.push(Trade {
            symbol: self.symbol.clone(),
            side: if t.long {
                TradeSide::Long
            } else {
                TradeSide::Short
            },
            entry_time: at(t.entry_ns),
            exit_time: at(at_ns),
            entry_price: entry,
            exit_price: exit,
            qty: t.entry_qty,
            mae,
            mfe,
            holding_period_secs: (at_ns - t.entry_ns) / 1_000_000_000,
            costs_paid: t.fees,
            pnl: t.realized - t.fees,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn fill(side: Side, qty: Decimal, price: Decimal, at_ns: i64) -> FillRecord {
        FillRecord {
            tag: "t".into(),
            side,
            qty,
            price,
            fee: dec!(1),
            at_ns,
        }
    }

    #[test]
    fn a_round_trip_is_flat_to_flat_net_of_both_fees() {
        let mut b = TradeBook::new("X");
        b.on_fill(&fill(Side::Buy, dec!(2), dec!(100), 0));
        b.on_bar(dec!(112), dec!(95));
        b.on_fill(&fill(Side::Sell, dec!(2), dec!(110), 3_600_000_000_000));
        let t = b.trades();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].pnl, dec!(18), "2 × 10 gained, 2 in fees");
        assert_eq!(t[0].holding_period_secs, 3_600);
        assert!(t[0].mae < 0.0 && t[0].mfe > 0.0);
    }

    #[test]
    fn a_flip_closes_one_trade_and_opens_the_other() {
        let mut b = TradeBook::new("X");
        b.on_fill(&fill(Side::Buy, dec!(1), dec!(100), 0));
        b.on_fill(&fill(Side::Sell, dec!(3), dec!(90), 1));
        b.on_fill(&fill(Side::Buy, dec!(2), dec!(80), 2));
        let t = b.trades();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].side, TradeSide::Long);
        assert_eq!(t[1].side, TradeSide::Short);
        assert_eq!(t[1].qty, dec!(2));
    }

    #[test]
    fn the_merge_puts_other_inputs_first_at_the_same_instant() {
        let b = |close_ns| Bar {
            close_ns,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        };
        let mut m = MergedBars::new(vec![vec![b(10), b(20)], vec![b(20)]], 10);
        let order: Vec<(usize, i64, bool)> = std::iter::from_fn(|| m.next_bar())
            .map(|e| (e.input, e.bar.close_ns, e.warmup))
            .collect();
        assert_eq!(order, vec![(0, 10, true), (1, 20, false), (0, 20, false)]);
    }
}

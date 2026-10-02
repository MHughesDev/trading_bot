//! [`SimulatedVenue`] — the backtest's venue (Set U U-2.2, ADR-0044).
//!
//! It is the paper venue core — the same [`PaperTradingEngine`], the same
//! per-asset-class accounts, the same fill simulators and therefore the same
//! fees, spreads and slippage — driven by historical bars on a [`SimClock`]
//! instead of live marks on the wall clock. That is the whole difference, and it
//! is the difference the owner asked for: backtest and paper work *similarly but
//! not exactly the same*, and they cost a trade identically by construction.
//!
//! The `market_simulator` crate stays as a reference venue to compare against;
//! it is no longer where backtests execute.
//!
//! ## One bar, in order
//!
//! A bar is knowable only at its close, so a strategy decides at bar `t`'s close
//! and its order cannot trade before bar `t+1`. Under [`FillPolicy::NextBarOpen`]
//! — the default and the only honest policy against OHLCV — each bar runs:
//!
//! 1. **The open.** The clock moves to the bar's open and the open is marked.
//!    Resting orders the market gapped through fill here, at the open.
//! 2. **Arrivals.** Orders submitted since the last bar reach the venue and are
//!    priced against the open: a market order fills at the open plus costs; a
//!    limit or stop that is already through fills or triggers; the rest rest.
//! 3. **The range.** [`PaperTradingEngine::on_bar_range`]: stops first against
//!    the bar's adverse extreme, then limits — so a bar spanning a bracket's stop
//!    and its target exits at the stop (FEAT-005 §3.4).
//! 4. **The close.** The clock moves to the close and the close is marked.
//!
//! [`FillPolicy::CurrentClose`] executes an order the moment it is submitted, at
//! the close the strategy just saw. It exists for comparison only: it trades on
//! a price nobody could have traded at, it is flagged optimistic, and the fill
//! policy is part of `RunConfig`'s hash so a run under it can never share an id
//! with an honest one.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use domain::{instrument::AssetClass, money::Price, order::OrderIntent};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::broker::{BrokerOrderState, BrokerOrderStatus, BrokerPosition};
use crate::clock::{SequentialIds, SimClock};
use crate::paper::{
    PaperAccountSnapshot, PaperEngineConfig, PaperTradeError, PaperTradingEngine, SimulatorSet,
};
use crate::venue_port::Venue;

/// When an order submitted at a bar's close first trades.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillPolicy {
    /// At the next bar's open, plus costs. The default.
    #[default]
    NextBarOpen,
    /// At the close the strategy decided on. **Optimistic**: comparison only.
    CurrentClose,
}

impl FillPolicy {
    /// Fills at a price nobody could have traded at.
    #[must_use]
    pub const fn is_optimistic(self) -> bool {
        matches!(self, Self::CurrentClose)
    }
}

/// One closed bar for one instrument.
#[derive(Clone, Debug, PartialEq)]
pub struct SimBar {
    pub instrument_id: String,
    pub open_time: DateTime<Utc>,
    pub close_time: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
}

/// An order the venue refused when it arrived.
#[derive(Debug)]
pub struct VenueRejection {
    pub key: Uuid,
    pub instrument_id: String,
    pub at: DateTime<Utc>,
    pub error: PaperTradeError,
}

/// What one bar did to the strategy's orders.
#[derive(Debug, Default)]
pub struct BarReport {
    /// Every order whose state or filled quantity changed, including orders that
    /// arrived this bar. Terminal orders are reported once.
    pub updates: Vec<BrokerOrderStatus>,
    /// Orders refused on arrival — first-class, never silently dropped.
    pub rejected: Vec<VenueRejection>,
}

/// The backtest's venue: the paper core on a simulated clock.
pub struct SimulatedVenue {
    engine: PaperTradingEngine,
    clock: Arc<SimClock>,
    policy: FillPolicy,
    /// Orders submitted since the last bar, in submission order.
    queue: Mutex<Vec<(AssetClass, OrderIntent)>>,
    /// Last reported (state, filled) per order the venue holds, by key. A
    /// `BTreeMap` so a report is in a deterministic order.
    seen: Mutex<BTreeMap<Uuid, (BrokerOrderState, Decimal)>>,
}

impl SimulatedVenue {
    /// A venue starting at `start`. `config` is the paper engine's own: pass
    /// `PaperEngineConfig::realistic()` to have session hours and mark freshness
    /// enforced against simulated time.
    #[must_use]
    pub fn new(
        starting_cash: &HashMap<AssetClass, Decimal>,
        sims: SimulatorSet,
        config: PaperEngineConfig,
        start: DateTime<Utc>,
        policy: FillPolicy,
    ) -> Self {
        let clock = Arc::new(SimClock::new(start));
        let engine = PaperTradingEngine::with_clock(
            starting_cash,
            sims,
            config,
            Arc::clone(&clock) as Arc<dyn crate::clock::VenueClock>,
            Arc::new(SequentialIds::default()),
        );
        Self {
            engine,
            clock,
            policy,
            queue: Mutex::new(Vec::new()),
            seen: Mutex::new(BTreeMap::new()),
        }
    }

    #[must_use]
    pub const fn policy(&self) -> FillPolicy {
        self.policy
    }

    /// The venue core, for reads the port does not carry (the journal, P&L).
    #[must_use]
    pub const fn engine(&self) -> &PaperTradingEngine {
        &self.engine
    }

    /// Run one bar. See the module docs for the order of events.
    pub fn on_bar(&self, bar: &SimBar) -> BarReport {
        self.on_bars(std::slice::from_ref(bar))
    }

    /// Run one timestamp's bars across instruments together, so the clock moves
    /// forward once: every open, then every arrival, every range, every close.
    pub fn on_bars(&self, bars: &[SimBar]) -> BarReport {
        let mut report = BarReport::default();
        let Some(open) = bars.iter().map(|b| b.open_time).min() else {
            return report;
        };
        let close = bars.iter().map(|b| b.close_time).max().unwrap_or(open);

        self.clock.set(open);
        for b in bars {
            self.engine
                .on_mark(&b.instrument_id, Price::from_decimal(b.open));
        }
        let due: Vec<(AssetClass, OrderIntent)> = {
            let mut q = self.queue.lock().expect("sim venue queue");
            let (due, keep) = std::mem::take(&mut *q)
                .into_iter()
                .partition(|(_, i)| bars.iter().any(|b| b.instrument_id == i.instrument_id));
            *q = keep;
            due
        };
        for (ac, intent) in due {
            self.arrive(ac, &intent, &mut report);
        }
        for b in bars {
            self.engine.on_bar_range(
                &b.instrument_id,
                Price::from_decimal(b.low),
                Price::from_decimal(b.high),
            );
        }
        self.clock.set(close);
        for b in bars {
            self.engine
                .on_mark(&b.instrument_id, Price::from_decimal(b.close));
        }
        report.updates = self.take_updates();
        report
    }

    /// Orders whose state or fill changed since the last call. `on_bar` calls
    /// this; under [`FillPolicy::CurrentClose`] call it after submitting to see
    /// the fills that happened at once.
    pub fn take_updates(&self) -> Vec<BrokerOrderStatus> {
        let mut seen = self.seen.lock().expect("sim venue seen");
        let mut out = Vec::new();
        let mut finished = Vec::new();
        for (key, last) in seen.iter_mut() {
            let Some(status) = self.status(*key) else {
                continue;
            };
            let now = (status.state.clone(), status.filled_qty);
            if *last != now {
                *last = now;
                out.push(status.clone());
            }
            if is_terminal(&status.state) {
                finished.push(*key);
            }
        }
        for k in finished {
            seen.remove(&k);
        }
        out
    }

    fn arrive(&self, asset_class: AssetClass, intent: &OrderIntent, report: &mut BarReport) {
        let key = intent.idempotency_key;
        // A sentinel no real status equals, so the arrival itself is reported.
        self.seen
            .lock()
            .expect("sim venue seen")
            .entry(key)
            .or_insert((
                BrokerOrderState::Unknown("arriving".into()),
                Decimal::NEGATIVE_ONE,
            ));
        if let Err(error) = self.engine.submit(asset_class, intent) {
            report.rejected.push(VenueRejection {
                key,
                instrument_id: intent.instrument_id.clone(),
                at: self.clock_now(),
                error,
            });
        }
    }

    fn status(&self, key: Uuid) -> Option<BrokerOrderStatus> {
        self.engine
            .order_id_for(key)
            .and_then(|id| self.engine.order_status(&id))
    }

    fn clock_now(&self) -> DateTime<Utc> {
        crate::clock::VenueClock::now(&*self.clock)
    }
}

fn is_terminal(s: &BrokerOrderState) -> bool {
    matches!(
        s,
        BrokerOrderState::Filled | BrokerOrderState::Cancelled | BrokerOrderState::Rejected
    )
}

impl Venue for SimulatedVenue {
    fn submit(
        &self,
        asset_class: AssetClass,
        intent: &OrderIntent,
    ) -> Result<Uuid, PaperTradeError> {
        let key = intent.idempotency_key;
        // A malformed order is refused now; it would be refused on arrival anyway.
        intent
            .check_shape()
            .map_err(|e| PaperTradeError::InvalidOrder(e.to_string()))?;
        match self.policy {
            FillPolicy::CurrentClose => {
                let mut report = BarReport::default();
                self.arrive(asset_class, intent, &mut report);
                if let Some(r) = report.rejected.pop() {
                    return Err(r.error);
                }
            }
            FillPolicy::NextBarOpen => {
                if self.engine.order_id_for(key).is_none() {
                    let mut q = self.queue.lock().expect("sim venue queue");
                    if !q.iter().any(|(_, i)| i.idempotency_key == key) {
                        q.push((asset_class, intent.clone()));
                    }
                }
            }
        }
        Ok(key)
    }

    fn cancel(&self, key: Uuid) -> Result<(), PaperTradeError> {
        {
            let mut q = self.queue.lock().expect("sim venue queue");
            let before = q.len();
            q.retain(|(_, i)| i.idempotency_key != key);
            if q.len() != before {
                return Ok(());
            }
        }
        let id = self
            .engine
            .order_id_for(key)
            .ok_or_else(|| PaperTradeError::UnknownOrder(key.to_string()))?;
        self.engine.cancel(&id)
    }

    fn order(&self, key: Uuid) -> Option<BrokerOrderStatus> {
        self.status(key)
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
        self.clock_now()
    }
}

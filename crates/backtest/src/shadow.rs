//! The backtest, run beside a paper deployment bar by bar — what the backtest
//! says should have happened over each live bar (Set T T-0.4; ADR-0047,
//! ADR-0048).
//!
//! Forward evidence compares a deployment's realised returns with an expected
//! series. A trial's stored backtest covers only its own historical window, so
//! it has nothing to say about a bar that closed after it. The expected value
//! for a live bar is therefore produced the only way that is comparable: the
//! same compiled plan, built from the same genesis as the paper engine, on the
//! backtest's [`SimulatedVenue`](execution::sim_venue::SimulatedVenue), fed the
//! identical bars, through [`crate::runner::step_bar`] — the one backtest loop
//! body. Realised against expected then measures the paper venue and the live
//! pipeline against the backtest model, and nothing else.
//!
//! The shadow hears the paper engine's model answers for the same tick, so a
//! model is never asked twice and both runs decide on the same answers.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use execution::paper::{PaperEngineConfig, SimulatorSet};
use execution::sim_risk::SimulatedRisk;
use execution::sim_venue::{FillPolicy, SimBar, SimulatedVenue};
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use strategy_engine::env::{MemoryJournal, ModelProvider};
use strategy_engine::plan::{InputIdx, PRIMARY};
use strategy_engine::{
    Bar, DecisionTrace, Engine, InstrumentSpec, ModelOutcome, ModelRequest, ModelResponse, Plan,
    RestoreError,
};

use crate::env::{at, SimOrders};
use crate::replay::Genesis;
use crate::runner::{step_bar, RunnerError};

/// What the shadow says about one primary bar.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowStep {
    /// The bar's close.
    pub bar: DateTime<Utc>,
    /// The shadow account's return over the bar: equity at this close over
    /// equity at the previous one, minus one.
    pub expected_return: f64,
    /// Notional the shadow traded during the bar.
    pub turnover: Decimal,
    /// Notional-weighted slippage of those fills against the price the
    /// shadow decided at (the previous close), in basis points.
    pub slippage_bps: f64,
    /// The shadow's decisions on this bar, its close included.
    pub decided: Vec<DecisionTrace>,
}

/// Model answers the paper engine received, replayed for the same tick.
#[derive(Default)]
struct PaperAnswers(BTreeMap<String, ModelOutcome>);

impl ModelProvider for PaperAnswers {
    fn answer(&mut self, req: &ModelRequest) -> ModelOutcome {
        self.0
            .remove(&req.node)
            .unwrap_or_else(|| ModelOutcome::Unavailable {
                detail: "the paper engine received no answer for this request".into(),
            })
    }
}

/// The backtest beside one deployment.
pub struct Shadow {
    engine: Engine,
    orders: SimOrders,
    instrument: String,
    /// The primary bar's width, ns: a live bar carries its close only.
    bar_ns: i64,
    /// Equity at the previous primary close.
    last_equity: f64,
    /// The previous primary close — the price an order decided on it fills
    /// against.
    last_close: Option<f64>,
    fills_seen: usize,
}

/// How to build a shadow.
pub struct ShadowSpec {
    pub plan: Arc<Plan>,
    pub spec: InstrumentSpec,
    pub asset_class: AssetClass,
    pub instrument_limits: InstrumentLimits,
    pub limits: GlobalRiskLimits,
    /// The account's opening cash: the deployment's capital, or its equity
    /// when the shadow is rebuilt flat after a restart.
    pub cash: Decimal,
    /// When the shadow's clock starts.
    pub start: DateTime<Utc>,
    pub strategy_id: String,
}

impl Shadow {
    /// A shadow whose engine is built from `genesis` exactly as the paper
    /// engine was, over a flat account holding `cash`.
    ///
    /// # Errors
    /// A genesis that does not build.
    pub fn new(s: ShadowSpec, genesis: &Genesis) -> Result<Self, RestoreError> {
        let engine = genesis.build(Arc::clone(&s.plan), &s.spec)?;
        let mut cash = HashMap::new();
        cash.insert(s.asset_class, s.cash);
        let venue = SimulatedVenue::new(
            &cash,
            SimulatorSet::realistic(),
            PaperEngineConfig::default(),
            s.start,
            FillPolicy::NextBarOpen,
        );
        let instrument = s.spec.instrument.clone();
        let bar_ns = i64::from(s.plan.inputs[PRIMARY].timeframe.seconds()) * 1_000_000_000;
        let mut instruments = BTreeMap::new();
        instruments.insert(instrument.clone(), s.instrument_limits);
        let risk = SimulatedRisk::new(s.limits, instruments, Some(s.strategy_id.clone()));
        let orders = SimOrders::new(venue, risk, s.asset_class, s.spec, s.strategy_id);
        Ok(Self {
            engine,
            orders,
            instrument,
            bar_ns,
            last_equity: s.cash.to_f64().unwrap_or(0.0),
            last_close: None,
            fills_seen: 0,
        })
    }

    /// The shadow account's equity at the latest marks.
    #[must_use]
    pub fn equity(&self) -> Decimal {
        self.orders.equity()
    }

    /// The shadow's position.
    #[must_use]
    pub fn position(&self) -> Decimal {
        self.orders.position()
    }

    /// One closed bar of `input`, as the paper engine heard it. `answers` are
    /// the model answers the paper engine received for this bar's decision.
    /// A primary bar is traded and decided, and returns the shadow's step.
    ///
    /// # Errors
    /// An engine defect.
    pub fn on_bar(
        &mut self,
        input: InputIdx,
        bar: Bar,
        answers: &[ModelResponse],
    ) -> Result<Option<ShadowStep>, RunnerError> {
        let mut models = PaperAnswers(
            answers
                .iter()
                .map(|r| (r.node.clone(), r.outcome.clone()))
                .collect(),
        );
        let sim = (input == PRIMARY).then(|| self.sim_bar(&bar));
        let decided = step_bar(
            &mut self.engine,
            input,
            bar,
            false,
            sim.as_ref(),
            &mut self.orders,
            &mut models,
            &mut MemoryJournal::default(),
        )?;
        if sim.is_none() {
            return Ok(None);
        }
        let new_fills = &self.orders.fills()[self.fills_seen..];
        self.fills_seen = self.orders.fills().len();
        let turnover: Decimal = new_fills.iter().map(|f| f.qty * f.price).sum();
        let slippage_bps = match self.last_close {
            Some(decided_at) if decided_at > 0.0 && !turnover.is_zero() => {
                let weighted: f64 = new_fills
                    .iter()
                    .map(|f| {
                        let px = f.price.to_f64().unwrap_or(decided_at);
                        let notional = (f.qty * f.price).to_f64().unwrap_or(0.0);
                        notional * ((px - decided_at).abs() / decided_at) * 1e4
                    })
                    .sum();
                weighted / turnover.to_f64().unwrap_or(1.0)
            }
            _ => 0.0,
        };
        let equity = self.orders.equity().to_f64().unwrap_or(0.0);
        let expected_return = if self.last_equity > 0.0 {
            equity / self.last_equity - 1.0
        } else {
            f64::NAN
        };
        self.last_equity = equity;
        self.last_close = Some(bar.close);
        Ok(Some(ShadowStep {
            bar: at(bar.close_ns),
            expected_return,
            turnover,
            slippage_bps,
            decided,
        }))
    }

    fn sim_bar(&self, b: &Bar) -> SimBar {
        // Through the shortest string that reads back as the same f64: a price
        // that was a decimal before it became a float is that decimal again,
        // so the venue trades the prices the backtest's store holds.
        let d = |x: f64| x.to_string().parse::<Decimal>().unwrap_or_default();
        SimBar {
            instrument_id: self.instrument.clone(),
            open_time: at(b.close_ns - self.bar_ns),
            close_time: at(b.close_ns),
            open: d(b.open),
            high: d(b.high),
            low: d(b.low),
            close: d(b.close),
        }
    }
}

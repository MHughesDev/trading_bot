//! Environment ports (Set U U-3.1, ADR-0039).
//!
//! An environment — backtest, preview, paper, live — is a set of adapters behind
//! these traits, and [`cycle`] is the one loop that drives the engine through
//! them. The engine depends on none of the adapters: it sees events, a
//! [`PortfolioView`] and an [`InstrumentSpec`], and answers with steps. What
//! changes between environments is only *where* bars, model answers, fills and
//! the time come from — never what happens to them.
//!
//! `cycle` is deliberately here, beside the engine, rather than in each
//! environment. A second copy of "answer the models, act on the decision, feed
//! the venue's refusals back" is a second place for backtest and paper to drift
//! apart, and drift is the defect this whole set exists to remove.

use std::collections::VecDeque;

use crate::engine::{Engine, EngineError, Event, Step};
use crate::history::Bar;
use crate::models::{ModelOutcome, ModelRequest, ModelResponse};
use crate::plan::InputIdx;
use crate::planner::OrderAction;
use crate::portfolio::{InstrumentSpec, PortfolioView};
use crate::trace::DecisionTrace;

/// Where the time comes from: the wall for paper and live, the bar being
/// replayed for a backtest.
pub trait Clock {
    /// Unix nanoseconds.
    fn now_ns(&self) -> i64;
}

/// One closed bar on one of the plan's inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarketEvent {
    pub input: InputIdx,
    pub bar: Bar,
    /// History before the evaluation window: builds indicators, never decides.
    pub warmup: bool,
}

/// Bars, in the order they closed, across every input the plan reads.
pub trait MarketData {
    /// The next closed bar, or `None` when the stream is exhausted.
    fn next_bar(&mut self) -> Option<MarketEvent>;
}

/// Answers model requests: a live endpoint, or — in a backtest — the
/// walk-forward fold that was trained early enough to answer at `req.at_ns`.
pub trait ModelProvider {
    fn answer(&mut self, req: &ModelRequest) -> ModelOutcome;
}

/// The engine's view of the venue: the account it reports, the instrument it
/// trades, and a place to carry out the planner's actions.
///
/// Named for the plan's `Venue` port; it is the strategy-side face of
/// `execution::Venue`, speaking planner tags rather than venue order ids.
pub trait VenuePort {
    /// The account and position at this instant.
    fn view(&self) -> PortfolioView;
    /// The bound instrument.
    fn spec(&self) -> InstrumentSpec;
    /// Carry out a decision's actions. Returns what the venue said
    /// *synchronously* — an order the risk gate refused comes back here as an
    /// [`Event::OrderClosed`] — so the engine learns of it before its next
    /// decision. Fills that happen later arrive through the environment's own
    /// loop.
    fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> Vec<Event>;
}

/// Where every decision is written down (ADR-0047).
pub trait Journal {
    fn record(&mut self, trace: &DecisionTrace);
}

/// A journal that keeps nothing: for callers that read the returned traces.
#[derive(Debug, Default)]
pub struct NoJournal;

impl Journal for NoJournal {
    fn record(&mut self, _trace: &DecisionTrace) {}
}

/// A journal that keeps every trace in memory.
#[derive(Debug, Default)]
pub struct MemoryJournal {
    pub traces: Vec<DecisionTrace>,
}

impl Journal for MemoryJournal {
    fn record(&mut self, trace: &DecisionTrace) {
        self.traces.push(trace.clone());
    }
}

/// Drive one event through the engine to completion.
///
/// Answers every model request the tick makes, journals the decision, hands its
/// actions to the venue, and feeds whatever the venue says back synchronously
/// (refusals) through the engine in turn, until nothing is left. Returns every
/// decision made, in order.
///
/// # Errors
/// An engine error — which, for a compiled plan, is an engine defect.
pub fn cycle(
    engine: &mut Engine,
    event: Event,
    models: &mut dyn ModelProvider,
    venue: &mut dyn VenuePort,
    journal: &mut dyn Journal,
) -> Result<Vec<DecisionTrace>, EngineError> {
    let mut queue = VecDeque::from([event]);
    let mut decided = Vec::new();
    while let Some(ev) = queue.pop_front() {
        let spec = venue.spec();
        let mut step = engine.on_event(ev, &venue.view(), &spec)?;
        while let Step::NeedModels(requests) = step {
            let responses = requests
                .iter()
                .map(|r| ModelResponse {
                    node: r.node.clone(),
                    outcome: models.answer(r),
                })
                .collect();
            step = engine.resume(responses)?;
        }
        if let Step::Decided(trace) = step {
            journal.record(&trace);
            if !trace.actions.is_empty() {
                queue.extend(venue.act(&trace.actions, trace.at_ns));
            }
            decided.push(*trace);
        }
    }
    Ok(decided)
}

/// A provider for environments with no models: every request is unavailable,
/// which the engine turns into `missing` and a hold — never a guess.
#[derive(Debug, Default)]
pub struct NoModels;

impl ModelProvider for NoModels {
    fn answer(&mut self, req: &ModelRequest) -> ModelOutcome {
        ModelOutcome::Unavailable {
            detail: format!(
                "no model provider in this environment for {}@{}",
                req.model, req.version
            ),
        }
    }
}

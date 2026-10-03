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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::DateTime;
use dataplane::universe::{MembershipChange, MembershipEvent, UniverseError};

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

// ── universes (U-6.1) ────────────────────────────────────────────────────────

/// Which instruments a selection strategy's universe holds, at any instant.
///
/// One type answers for every environment: a backtest asks it at each decision
/// instant, a live allocator asks it *now*, and both get their answer from the
/// same code over the same definition — so a universe cannot hold one set of
/// members in a backtest and another on paper for the same instant.
///
/// A **logged** universe (`dataplane::universe`) is point-in-time on both of its
/// clocks: at `t` it holds what had taken effect by `t` *and* had been recorded
/// by `t`. An instrument added later is not there; one removed since still is.
/// A **fixed** universe — an explicit list, or every initialised asset of a
/// class — is today's membership applied to every instant. It cannot know what
/// was delisted before it was written down, and says so: [`Self::point_in_time`]
/// is `false`, and a run over it records the survivorship caveat.
#[derive(Clone, Debug, PartialEq)]
pub struct Membership {
    source: MembershipSource,
}

#[derive(Clone, Debug, PartialEq)]
enum MembershipSource {
    Fixed(Vec<String>),
    Logged {
        log: Vec<MembershipEvent>,
        /// Instrument key → the symbol bars are stored under.
        symbols: BTreeMap<i64, String>,
    },
}

fn symbol_of(symbols: &BTreeMap<i64, String>, key: i64) -> String {
    symbols
        .get(&key)
        .cloned()
        .unwrap_or_else(|| format!("#{key}"))
}

impl Membership {
    /// The same members at every instant: a list, or an asset class's
    /// initialised instruments as they are today.
    #[must_use]
    pub fn fixed(instruments: impl IntoIterator<Item = String>) -> Self {
        let set: BTreeSet<String> = instruments.into_iter().collect();
        Self {
            source: MembershipSource::Fixed(set.into_iter().collect()),
        }
    }

    /// A universe resolved from its append-only membership log. `symbols` names
    /// each instrument key; a key without a symbol is still a member — under
    /// `#<key>`, so it is excluded for having no data rather than silently
    /// dropped, which would shrink the universe the way survivorship does.
    ///
    /// # Errors
    /// A log that is not internally consistent (`dataplane::universe`).
    pub fn logged(
        log: Vec<MembershipEvent>,
        symbols: BTreeMap<i64, String>,
    ) -> Result<Self, UniverseError> {
        dataplane::universe::intervals(&log)?;
        Ok(Self {
            source: MembershipSource::Logged { log, symbols },
        })
    }

    /// The members at `at_ns`, sorted.
    ///
    /// # Errors
    /// A log that cannot be resolved at that instant.
    pub fn members_at(&self, at_ns: i64) -> Result<Vec<String>, UniverseError> {
        match &self.source {
            MembershipSource::Fixed(xs) => Ok(xs.clone()),
            MembershipSource::Logged { log, symbols } => {
                let t = DateTime::from_timestamp_nanos(at_ns);
                let keys = dataplane::universe::members_known_as_of(log, t, t)?;
                let out: BTreeSet<String> =
                    keys.into_iter().map(|k| symbol_of(symbols, k.0)).collect();
                Ok(out.into_iter().collect())
            }
        }
    }

    /// Every instrument that is a member at any instant in `[from_ns, to_ns]` —
    /// what a backtest must load bars for.
    #[must_use]
    pub fn ever_between(&self, from_ns: i64, to_ns: i64) -> Vec<String> {
        match &self.source {
            MembershipSource::Fixed(xs) => xs.clone(),
            MembershipSource::Logged { log, symbols } => {
                let from = DateTime::from_timestamp_nanos(from_ns);
                let to = DateTime::from_timestamp_nanos(to_ns);
                let mut out = BTreeSet::new();
                // Members at the start, plus every addition that becomes known
                // inside the window.
                if let Ok(start) = dataplane::universe::members_known_as_of(log, from, from) {
                    out.extend(start.into_iter().map(|k| symbol_of(symbols, k.0)));
                }
                for e in log {
                    let known = e.effective.max(e.recorded_at);
                    if e.change == MembershipChange::Added && known > from && known <= to {
                        out.insert(symbol_of(symbols, e.instrument_id.0));
                    }
                }
                out.into_iter().collect()
            }
        }
    }

    /// Whether membership is known as of each instant. `false` for a fixed
    /// universe: today's list, applied to the past.
    #[must_use]
    pub const fn point_in_time(&self) -> bool {
        matches!(self.source, MembershipSource::Logged { .. })
    }
}

#[cfg(test)]
mod universe_tests {
    use super::Membership;
    use chrono::{TimeZone, Utc};
    use dataplane::universe::MembershipEvent;
    use dataplane::InstrumentKey;
    use std::collections::BTreeMap;

    fn day(d: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, d, 0, 0, 0).unwrap()
    }

    fn ns(d: u32) -> i64 {
        day(d).timestamp_nanos_opt().unwrap()
    }

    fn universe() -> Membership {
        let k = InstrumentKey;
        let log = vec![
            MembershipEvent::added("u", k(1), day(1), "seed"),
            MembershipEvent::added("u", k(3), day(1), "seed"),
            MembershipEvent::removed("u", k(3), day(4), "delisted"),
            MembershipEvent::added("u", k(2), day(6), "listed"),
            // Took effect on the 2nd, but nobody knew until the 8th.
            MembershipEvent::added("u", k(4), day(2), "late").known_at(day(8)),
            MembershipEvent::added("u", k(9), day(1), "no symbol"),
        ];
        let symbols = BTreeMap::from([
            (1, "A".to_string()),
            (2, "B".to_string()),
            (3, "C".to_string()),
            (4, "D".to_string()),
        ]);
        Membership::logged(log, symbols).unwrap()
    }

    #[test]
    fn at_t_a_universe_excludes_later_additions_and_includes_later_removals() {
        let u = universe();
        assert!(u.point_in_time());
        assert_eq!(
            u.members_at(ns(3)).unwrap(),
            ["#9", "A", "C"],
            "B is added on the 6th and is not here; C is removed on the 4th and still is; \
             a key with no symbol stays a member under its key"
        );
        assert_eq!(u.members_at(ns(5)).unwrap(), ["#9", "A"]);
        assert_eq!(
            u.members_at(ns(7)).unwrap(),
            ["#9", "A", "B"],
            "D took effect on the 2nd but was not known until the 8th"
        );
        assert_eq!(u.members_at(ns(9)).unwrap(), ["#9", "A", "B", "D"]);
        assert_eq!(u.ever_between(ns(3), ns(9)), ["#9", "A", "B", "C", "D"]);
        assert_eq!(u.ever_between(ns(5), ns(6)), ["#9", "A", "B"]);
    }

    #[test]
    fn a_fixed_universe_is_the_same_at_every_instant_and_says_it_is_not_point_in_time() {
        let u = Membership::fixed(["B".to_string(), "A".to_string(), "A".to_string()]);
        assert!(!u.point_in_time());
        assert_eq!(u.members_at(0).unwrap(), ["A", "B"]);
        assert_eq!(u.members_at(ns(9)).unwrap(), u.members_at(0).unwrap());
    }
}

//! Portfolio backtests: a trading strategy deployed on a selection (Set U U-6.5;
//! FEAT-007 §4; ADR-0040 decision 4, ADR-0044).
//!
//! A selection-bound deployment trades whatever its selection picks. Its
//! backtest is one simulated clock and **one account** across every instrument
//! that is ever a member: every bar of every member merged in time order, the
//! selection re-run on its own timeframe over the universe *as it was at that
//! instant*, and the rebalance applied through
//! [`strategy_engine::allocation::rebalance`] — the same function the live
//! allocator calls. Each member trades through its own trading instance (the
//! one strategy engine, driven by [`strategy_engine::cycle`]) and its own sleeve
//! of the shared account.
//!
//! # One instant
//!
//! At each close instant `T`, in this order:
//!
//! 1. every member's trading bar for `T` runs through the venue, and each running
//!    instance hears of its fills before it decides;
//! 2. bars closing at `T` that are not a trading decision bar build history —
//!    the selection's, and the instances' other inputs;
//! 3. if the selection's timeframe closes at `T`, it is evaluated over the
//!    members at `T` and the deployment rebalances: entrants start an instance
//!    (warmed on their own history before `T`), leavers are closed, held or left
//!    to exit, stayers are reweighted;
//! 4. every running instance decides on its `T` bar;
//! 5. an instance letting its strategy exit stops once it is flat with nothing
//!    working, and its weight is released;
//! 6. equity and exposure are sampled once.
//!
//! An instrument therefore trades only while it holds an allocation — entering at
//! the rebalance that admits it, flattened at the one that drops it (`close`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use domain::strategy_graph::deployment::{OnRemoval, Rebalance, Weighting};
use domain::strategy_graph::selection::Selection;
use execution::paper::{PaperEngineConfig, SimulatorSet};
use execution::sim_risk::SimulatedRisk;
use execution::sim_venue::{FillPolicy, SimulatedVenue};
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use strategy_engine::allocation::{self, Allocation, AllocationChange};
use strategy_engine::env::{Journal, MemoryJournal, ModelProvider};
use strategy_engine::plan::PRIMARY;
use strategy_engine::schedule::Cron;
use strategy_engine::selection::{SelectionEngine, SelectionStep};
use strategy_engine::{
    cycle, DecisionTrace, Engine, EngineError, Event, InstrumentSpec, Membership, ModelResponse,
    Plan,
};

use crate::env::{at, AttributedTrade, SimOrders};
use crate::run::result::GateRejectionRecord;
use crate::store::LoadedBar;

/// Everything one portfolio run needs, already loaded.
pub struct PortfolioRun {
    /// The trading strategy each member runs.
    pub trading: Arc<Plan>,
    /// The selection that decides the members.
    pub selection: Arc<Plan>,
    pub membership: Membership,
    pub top_n: u32,
    pub on_removal: OnRemoval,
    pub weighting: Weighting,
    /// When membership is re-applied: after every selection run, or on a
    /// schedule of its own (using the latest selection).
    pub rebalance: Rebalance,
    /// Instrument → bars for each trading-plan input, warm-up included.
    pub trading_bars: BTreeMap<String, Vec<Vec<LoadedBar>>>,
    /// Instrument → bars for each selection-plan input, warm-up included.
    pub selection_bars: BTreeMap<String, Vec<Vec<LoadedBar>>>,
    /// Bars closing at or before this are warm-up.
    pub start_ns: i64,
    /// What each instrument trades as, and the gate's view of it.
    pub specs: BTreeMap<String, (InstrumentSpec, InstrumentLimits)>,
    pub asset_class: AssetClass,
    pub limits: GlobalRiskLimits,
    /// The deployment's capital; weights are fractions of it.
    pub capital: Decimal,
    pub fill_policy: FillPolicy,
    pub strategy_id: String,
    pub sims: SimulatorSet,
}

/// What a portfolio run produced.
pub struct PortfolioOutcome {
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub net_exposure: Vec<(DateTime<Utc>, f64)>,
    /// Every round trip, with the instance that opened it.
    pub trades: Vec<AttributedTrade>,
    /// Realised P&L net of costs, summed per quote currency — never across two.
    pub pnl_by_numeraire: BTreeMap<String, Decimal>,
    pub final_equity: Decimal,
    /// Every in-window selection, in order.
    pub selections: Vec<Selection>,
    /// Every allocation change, with its instant.
    pub changes: Vec<(i64, AllocationChange)>,
    /// Every decision inside the window, with the instance that made it.
    pub traces: Vec<(String, DecisionTrace)>,
    /// Entrants that could not be started, and why.
    pub skipped: Vec<(i64, String, String)>,
    /// `false`: the universe is today's list applied to the past — the run
    /// carries a survivorship caveat.
    pub universe_point_in_time: bool,
    pub gate_rejections: Vec<GateRejectionRecord>,
}

#[derive(Debug, thiserror::Error)]
pub enum PortfolioError {
    #[error("the trading plan is not a trading strategy")]
    NotTrading,
    #[error("the selection plan is not a selection strategy")]
    NotSelection,
    #[error("the trading strategy's scheduled trigger ('{0}') is not driven in portfolio backtests yet; schedule the selection or the rebalance instead")]
    ScheduleNotDriven(String),
    #[error("schedule: {0}")]
    Schedule(String),
    #[error("{instrument}: {got} bar series for a plan that reads {want}")]
    InputCount {
        instrument: String,
        got: usize,
        want: usize,
    },
    #[error("universe: {0}")]
    Universe(#[from] dataplane::universe::UniverseError),
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
}

/// One bar, as the merged clock carries it.
#[derive(Clone, Copy)]
enum Feed {
    Trading,
    Selection,
}

struct Running {
    instance: String,
    engine: Engine,
    exiting: bool,
}

fn engine_bar(b: &LoadedBar) -> strategy_engine::Bar {
    use rust_decimal::prelude::ToPrimitive;
    let f = |d: Decimal| d.to_f64().unwrap_or(f64::NAN);
    strategy_engine::Bar {
        close_ns: b.ts_ns,
        open: f(b.open),
        high: f(b.high),
        low: f(b.low),
        close: f(b.close),
        volume: f(b.volume),
    }
}

fn sim_bar(instrument: &str, b: &LoadedBar) -> execution::sim_venue::SimBar {
    execution::sim_venue::SimBar {
        instrument_id: instrument.to_string(),
        open_time: at(b.open_ns),
        close_time: at(b.ts_ns),
        open: b.open,
        high: b.high,
        low: b.low,
        close: b.close,
    }
}

/// The quote currency a symbol's P&L is denominated in: `ETH-BTC` → `BTC`. A
/// symbol with no quote is in the account's own currency.
fn numeraire(symbol: &str) -> String {
    symbol
        .rsplit_once(['-', '/'])
        .map_or_else(|| "USD".to_string(), |(_, q)| q.to_string())
}

/// Run a selection-bound deployment over loaded bars.
///
/// # Errors
/// Plans of the wrong kind, bars that do not match a plan, a scheduled
/// selection, an inconsistent universe, or an engine defect.
#[allow(clippy::too_many_lines)]
pub fn run_portfolio(
    r: PortfolioRun,
    models: &mut dyn ModelProvider,
) -> Result<PortfolioOutcome, PortfolioError> {
    use domain::strategy_graph::StrategyKind;
    if r.trading.kind != StrategyKind::Trading {
        return Err(PortfolioError::NotTrading);
    }
    if r.selection.kind != StrategyKind::Selection {
        return Err(PortfolioError::NotSelection);
    }
    if let Some(s) = &r.trading.triggers.schedule {
        return Err(PortfolioError::ScheduleNotDriven(s.clone()));
    }
    let selection_cron = r
        .selection
        .triggers
        .schedule
        .as_deref()
        .map(Cron::parse)
        .transpose()
        .map_err(PortfolioError::Schedule)?;
    let rebalance_cron = match &r.rebalance {
        Rebalance::OnSelectionRun => None,
        Rebalance::Schedule { cron } => Some(Cron::parse(cron).map_err(PortfolioError::Schedule)?),
    };
    for (m, bars) in &r.trading_bars {
        if bars.len() != r.trading.inputs.len() {
            return Err(PortfolioError::InputCount {
                instrument: m.clone(),
                got: bars.len(),
                want: r.trading.inputs.len(),
            });
        }
    }
    for (m, bars) in &r.selection_bars {
        if bars.len() != r.selection.inputs.len() {
            return Err(PortfolioError::InputCount {
                instrument: m.clone(),
                got: bars.len(),
                want: r.selection.inputs.len(),
            });
        }
    }

    // The account.
    let first_open = r
        .trading_bars
        .values()
        .filter_map(|b| b[PRIMARY].iter().find(|x| x.ts_ns > r.start_ns))
        .map(|b| b.open_ns)
        .min()
        .unwrap_or(r.start_ns);
    let mut cash = std::collections::HashMap::new();
    cash.insert(r.asset_class, r.capital);
    let venue = SimulatedVenue::new(
        &cash,
        r.sims,
        PaperEngineConfig::default(),
        at(first_open),
        r.fill_policy,
    );
    let limits: BTreeMap<String, InstrumentLimits> = r
        .specs
        .iter()
        .map(|(m, (_, l))| (m.clone(), *l))
        .collect();
    let risk = SimulatedRisk::new(r.limits.clone(), limits, Some(r.strategy_id.clone()));
    let mut orders = SimOrders::portfolio(venue, risk, r.asset_class, r.strategy_id.clone());

    // One clock: every bar of every member, by close instant.
    let mut by_instant: BTreeMap<i64, Vec<(String, Feed, usize, usize)>> = BTreeMap::new();
    for (feed, all) in [
        (Feed::Trading, &r.trading_bars),
        (Feed::Selection, &r.selection_bars),
    ] {
        for (m, inputs) in all {
            for (input, bars) in inputs.iter().enumerate() {
                for (i, b) in bars.iter().enumerate() {
                    by_instant
                        .entry(b.ts_ns)
                        .or_default()
                        .push((m.clone(), feed, input, i));
                }
            }
        }
    }

    // Scheduled instants are instants of the clock too, bars or not.
    if let (Some(&first), Some(&last)) = (by_instant.keys().next(), by_instant.keys().next_back()) {
        for c in [&selection_cron, &rebalance_cron].into_iter().flatten() {
            for t in c.fires_between(first - 1, last) {
                by_instant.entry(t).or_default();
            }
        }
    }

    let mut selector = SelectionEngine::new(Arc::clone(&r.selection))?;
    let mut latest: Option<Selection> = None;
    let mut running: BTreeMap<String, Running> = BTreeMap::new();
    let mut alloc = Allocation::new();
    let mut started: BTreeMap<String, u32> = BTreeMap::new();
    let mut journal = MemoryJournal::default();
    let mut out_traces: Vec<(String, DecisionTrace)> = Vec::new();
    let mut selections = Vec::new();
    let mut changes: Vec<(i64, AllocationChange)> = Vec::new();
    let mut skipped = Vec::new();

    for (&t, bars) in &by_instant {
        let in_window = t > r.start_ns;
        let trading_primary: BTreeSet<&str> = bars
            .iter()
            .filter(|(_, f, input, _)| matches!(f, Feed::Trading) && *input == PRIMARY)
            .map(|(m, ..)| m.as_str())
            .collect();

        // 1. the venue trades each member's bar; instances hear of their fills.
        if in_window {
            for m in &trading_primary {
                let b = &r.trading_bars[*m][PRIMARY];
                let idx = b.iter().position(|x| x.ts_ns == t).expect("indexed above");
                let events = orders.on_bar_of(&sim_bar(m, &b[idx]));
                if let Some(run) = running.get_mut(*m) {
                    for e in events {
                        let decided = cycle(
                            &mut run.engine,
                            e,
                            models,
                            &mut orders.port(m),
                            &mut journal,
                        )?;
                        out_traces.extend(decided.into_iter().map(|d| (run.instance.clone(), d)));
                    }
                }
            }
        }

        // 2. history that is not a trading decision.
        for (m, feed, input, i) in bars {
            match feed {
                Feed::Selection => {
                    selector.on_bar(m, *input, engine_bar(&r.selection_bars[m][*input][*i]))?;
                }
                Feed::Trading if *input != PRIMARY => {
                    if let Some(run) = running.get_mut(m) {
                        let ev = Event::Bar {
                            input: *input,
                            bar: engine_bar(&r.trading_bars[m][*input][*i]),
                            warmup: !in_window,
                        };
                        cycle(
                            &mut run.engine,
                            ev,
                            models,
                            &mut orders.port(m),
                            &mut journal,
                        )?;
                    }
                }
                Feed::Trading => {}
            }
        }

        // 3. the selection, and the rebalance it drives.
        let selection_closes = r.selection.triggers.bar_close
            && bars
                .iter()
                .any(|(_, f, input, _)| matches!(f, Feed::Selection) && *input == PRIMARY);
        let selection_due =
            selection_closes || selection_cron.as_ref().is_some_and(|c| c.fires_at(t));
        let mut fresh = false;
        if selection_due {
            let members = r.membership.members_at(t)?;
            let mut step = selector.evaluate(t, &members, !in_window)?;
            while let SelectionStep::NeedModels(reqs) = step {
                let answers = reqs
                    .iter()
                    .map(|q| ModelResponse {
                        node: q.node.clone(),
                        outcome: models.answer(q),
                    })
                    .collect();
                step = selector.resume(answers)?;
            }
            if let SelectionStep::Selected(run) = step {
                selections.push(run.selection.clone());
                latest = Some(run.selection);
                fresh = true;
            }
        }
        let rebalance_due = match &rebalance_cron {
            None => fresh,
            Some(c) => in_window && c.fires_at(t),
        };
        if rebalance_due {
            if let Some(selection) = latest.as_ref() {
                let rb =
                    allocation::rebalance(&alloc, selection, r.top_n, r.on_removal, &r.weighting);
                let mut next = rb.next;
                for change in rb.changes {
                    match &change {
                        AllocationChange::Entered { instrument, weight } => {
                            let Some((spec, _)) = r.specs.get(instrument) else {
                                skipped.push((
                                    t,
                                    instrument.clone(),
                                    "no instrument metadata".into(),
                                ));
                                next.remove(instrument);
                                continue;
                            };
                            let Some(series) = r.trading_bars.get(instrument) else {
                                skipped.push((t, instrument.clone(), "no bars".into()));
                                next.remove(instrument);
                                continue;
                            };
                            let k = started.entry(instrument.clone()).or_insert(0);
                            *k += 1;
                            let instance = format!("{instrument}#{k}");
                            orders.open_sleeve(spec.clone(), instance.clone(), r.capital * *weight);
                            let mut engine = Engine::new(Arc::clone(&r.trading))?;
                            warm(
                                &mut engine,
                                series,
                                t,
                                models,
                                &mut orders,
                                instrument,
                                &mut journal,
                            )?;
                            running.insert(
                                instrument.clone(),
                                Running {
                                    instance,
                                    engine,
                                    exiting: false,
                                },
                            );
                        }
                        AllocationChange::Left { instrument, action } => {
                            running.remove(instrument);
                            if *action == OnRemoval::Close {
                                orders.flatten(instrument, t);
                            }
                        }
                        AllocationChange::Exiting { instrument, .. } => {
                            if let Some(run) = running.get_mut(instrument) {
                                run.exiting = true;
                            }
                        }
                        AllocationChange::Reweighted { instrument, to, .. } => {
                            orders.set_capital(instrument, r.capital * *to);
                            if let Some(run) = running.get_mut(instrument) {
                                run.exiting = false;
                            }
                        }
                    }
                    changes.push((t, change));
                }
                alloc = next;
            }
        }

        // 4. every running instance decides on its bar.
        if in_window {
            for m in &trading_primary {
                let Some(run) = running.get_mut(*m) else {
                    continue;
                };
                let b = &r.trading_bars[*m][PRIMARY];
                let idx = b.iter().position(|x| x.ts_ns == t).expect("indexed above");
                let ev = Event::Bar {
                    input: PRIMARY,
                    bar: engine_bar(&b[idx]),
                    warmup: false,
                };
                let decided = cycle(
                    &mut run.engine,
                    ev,
                    models,
                    &mut orders.port(m),
                    &mut journal,
                )?;
                out_traces.extend(decided.into_iter().map(|d| (run.instance.clone(), d)));
            }

            // 5. exits that have finished.
            let done: Vec<String> = running
                .iter()
                .filter(|(m, run)| run.exiting && orders.is_idle(m))
                .map(|(m, _)| m.clone())
                .collect();
            for m in done {
                running.remove(&m);
                if let Some(c) = allocation::finish_exit(&mut alloc, &m) {
                    changes.push((t, c));
                }
            }

            // 6. one sample per decision instant.
            if !trading_primary.is_empty() {
                orders.sample(t);
            }
        }
    }

    let universe_point_in_time = r.membership.point_in_time();
    let results = orders.into_portfolio_results();
    let mut pnl_by_numeraire: BTreeMap<String, Decimal> = BTreeMap::new();
    for t in &results.trades {
        *pnl_by_numeraire
            .entry(numeraire(&t.trade.symbol))
            .or_default() += t.trade.pnl;
    }
    Ok(PortfolioOutcome {
        equity: results.equity,
        net_exposure: results.net_exposure,
        trades: results.trades,
        pnl_by_numeraire,
        final_equity: results.final_equity,
        selections,
        changes,
        traces: out_traces,
        skipped,
        universe_point_in_time,
        gate_rejections: results.gate_rejections,
    })
}

/// Bring a new instance up to `t` on its instrument's own history: every bar
/// before `t` as warm-up, and the other inputs' bars at `t` (which have closed
/// by the decision at `t`).
fn warm(
    engine: &mut Engine,
    series: &[Vec<LoadedBar>],
    t: i64,
    models: &mut dyn ModelProvider,
    orders: &mut SimOrders,
    instrument: &str,
    journal: &mut dyn Journal,
) -> Result<(), EngineError> {
    let mut events: Vec<(i64, bool, usize, &LoadedBar)> = Vec::new();
    for (input, bars) in series.iter().enumerate() {
        for b in bars {
            if b.ts_ns < t || (b.ts_ns == t && input != PRIMARY) {
                events.push((b.ts_ns, input == PRIMARY, input, b));
            }
        }
    }
    // At equal instants other inputs come first, as in a single-instrument run.
    events.sort_by_key(|(ts, primary, input, _)| (*ts, *primary, *input));
    for (ts, _, input, b) in events {
        let ev = Event::Bar {
            input,
            bar: engine_bar(b),
            warmup: ts < t,
        };
        cycle(engine, ev, models, &mut orders.port(instrument), journal)?;
    }
    Ok(())
}

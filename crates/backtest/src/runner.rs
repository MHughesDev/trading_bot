//! `BacktestRunner` — v3 strategies backtest through the one strategy engine
//! (Set U U-3.2, U-3.3; FEAT-007 §9.4; ADR-0039, ADR-0044, ADR-0046).
//!
//! A v3 run is the engine meeting history through the backtest environment's
//! ports ([`crate::env`]): every input's bars in the order they closed, the
//! simulated venue that shares the paper venue's core, the production risk gate
//! with simulated inputs, and a journal of every decision. The loop is
//! [`strategy_engine::cycle`] — the same function paper drives — so the only
//! thing a backtest and a paper session do differently is where their bars,
//! fills and time come from.
//!
//! Two layers:
//!
//! - [`run`] — pure: a compiled plan, bars already loaded, out comes a
//!   [`V3Outcome`]. No database, no clock, no network. Tests and preview use it
//!   directly.
//! - [`V3RunExecutor`] — a [`RunExecutor`]: resolves the revision, loads every
//!   declared (instrument, timeframe) point-in-time, calls [`run`], and maps the
//!   outcome into the [`RunResult`] Set J's funnel, studies and nulls already
//!   judge. Being a `RunExecutor` is what puts every v3 run in the trial ledger
//!   before it computes: no executor is reachable without a `TrialTicket`
//!   (INV-16).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use domain::strategy_graph::StrategyGraph;
use execution::paper::{PaperEngineConfig, SimulatorSet};
use execution::sim_risk::SimulatedRisk;
use execution::sim_venue::{FillPolicy, SimBar, SimulatedVenue};
use ledger::TerminalReason;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use strategy_engine::decide::RuleOutcome;
use strategy_engine::env::{Journal, MarketData, MemoryJournal, ModelProvider};
use strategy_engine::plan::{NodeKind, PRIMARY};
use strategy_engine::trace::Cause;
use strategy_engine::{
    cycle, Bar, DecisionTrace, Engine, EngineError, Event, InstrumentSpec, Plan,
};

use crate::env::{at, BarVenue, MergedBars, SimOrders};
use crate::run::executor::map_sim_result;
use crate::run::result::{
    ComputeCost, EngineManifest, GateRejectionRecord, JournalEntry, ModelPin, RunResult, Trade,
};
use crate::run::{FillModel, RunConfig, RunExecutor};
use crate::store::LoadedBar;

/// `produced_by` on every result this runner emits.
pub const RUNNER_TAG: &str = "BacktestRunner@strategy-engine/v3";

/// The engine identifier in a manifest.
pub const ENGINE_ID: &str = "strategy-engine/v3";

/// Everything one v3 run needs, already loaded.
pub struct V3Run {
    pub plan: Arc<Plan>,
    /// The instrument the strategy is bound to.
    pub instrument: String,
    pub asset_class: AssetClass,
    /// `bars[i]` are plan input `i`'s bars, warm-up included, ordered by close.
    pub bars: Vec<Vec<LoadedBar>>,
    /// The evaluation window, Unix ns. Bars closing at or before `start_ns` are
    /// warm-up.
    pub start_ns: i64,
    pub spec: InstrumentSpec,
    pub instrument_limits: InstrumentLimits,
    pub limits: GlobalRiskLimits,
    pub initial_balance: Decimal,
    pub fill_policy: FillPolicy,
    /// What the strategy's orders carry as `strategy_id`.
    pub strategy_id: String,
    pub sims: SimulatorSet,
}

/// What a v3 run produced.
pub struct V3Outcome {
    /// Equity at every primary bar close inside the window.
    pub equity: Vec<(DateTime<Utc>, f64)>,
    pub net_exposure: Vec<(DateTime<Utc>, f64)>,
    pub trades: Vec<Trade>,
    /// Every decision inside the window, in order.
    pub traces: Vec<DecisionTrace>,
    pub manifest: EngineManifest,
}

/// Why a v3 run could not run.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("the plan reads {plan} inputs but {given} bar series were supplied")]
    InputCount { plan: usize, given: usize },
    #[error("no bars for the bound instrument inside the window")]
    NoPrimaryBars,
    #[error("scheduled triggers ('{0}') are not driven in backtests yet; use bar-close or on-fill triggers")]
    ScheduleNotDriven(String),
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
}

fn engine_bar(b: &LoadedBar) -> Bar {
    let f = |d: Decimal| d.to_f64().unwrap_or(f64::NAN);
    Bar {
        close_ns: b.ts_ns,
        open: f(b.open),
        high: f(b.high),
        low: f(b.low),
        close: f(b.close),
        volume: f(b.volume),
    }
}

fn sim_bar(instrument: &str, b: &LoadedBar) -> SimBar {
    SimBar {
        instrument_id: instrument.to_string(),
        open_time: at(b.open_ns),
        close_time: at(b.ts_ns),
        open: b.open,
        high: b.high,
        low: b.low,
        close: b.close,
    }
}

/// Run a compiled v3 plan over loaded bars.
///
/// # Errors
/// Inputs that do not match the plan, no primary bars in the window, a
/// scheduled trigger, or an engine defect.
pub fn run(r: V3Run, models: &mut dyn ModelProvider) -> Result<V3Outcome, RunnerError> {
    run_with_journal(r, models, &mut MemoryJournal::default())
}

/// [`run`] writing every decision to `journal` as it is made.
///
/// # Errors
/// As [`run`].
pub fn run_with_journal(
    r: V3Run,
    models: &mut dyn ModelProvider,
    journal: &mut dyn Journal,
) -> Result<V3Outcome, RunnerError> {
    let primary = primary_bars(&r)?;
    let first = primary.values().next().ok_or(RunnerError::NoPrimaryBars)?;

    let mut cash = HashMap::new();
    cash.insert(r.asset_class, r.initial_balance);
    let venue = SimulatedVenue::new(
        &cash,
        r.sims,
        PaperEngineConfig::default(),
        first.open_time,
        r.fill_policy,
    );
    let mut instruments = BTreeMap::new();
    instruments.insert(r.instrument.clone(), r.instrument_limits);
    let risk = SimulatedRisk::new(r.limits.clone(), instruments, Some(r.strategy_id.clone()));
    let mut orders = SimOrders::new(venue, risk, r.asset_class, r.spec, r.strategy_id.clone());

    let traces = drive(
        &r.plan,
        &r.bars,
        &primary,
        r.start_ns,
        &mut orders,
        models,
        journal,
    )?;
    let results = orders.into_results();
    let manifest = manifest(
        &r.plan,
        r.fill_policy,
        &r.limits,
        &traces,
        results.gate_rejections,
    );
    Ok(V3Outcome {
        equity: results.equity,
        net_exposure: results.net_exposure,
        trades: results.trades,
        traces,
        manifest,
    })
}

/// The window's primary bars as the venue sees them, by close.
pub(crate) fn primary_bars(r: &V3Run) -> Result<BTreeMap<i64, SimBar>, RunnerError> {
    if r.bars.len() != r.plan.inputs.len() {
        return Err(RunnerError::InputCount {
            plan: r.plan.inputs.len(),
            given: r.bars.len(),
        });
    }
    if let Some(s) = &r.plan.triggers.schedule {
        return Err(RunnerError::ScheduleNotDriven(s.clone()));
    }
    let primary: BTreeMap<i64, SimBar> = r.bars[PRIMARY]
        .iter()
        .filter(|b| b.ts_ns > r.start_ns)
        .map(|b| (b.ts_ns, sim_bar(&r.instrument, b)))
        .collect();
    if primary.is_empty() {
        return Err(RunnerError::NoPrimaryBars);
    }
    Ok(primary)
}

/// The one backtest loop, over any venue. Returns every decision inside the
/// window.
pub(crate) fn drive(
    plan: &Arc<Plan>,
    bars: &[Vec<LoadedBar>],
    primary: &BTreeMap<i64, SimBar>,
    start_ns: i64,
    venue: &mut dyn BarVenue,
    models: &mut dyn ModelProvider,
    journal: &mut dyn Journal,
) -> Result<Vec<DecisionTrace>, RunnerError> {
    let mut engine = Engine::new(Arc::clone(plan))?;
    let mut data = MergedBars::new(
        bars.iter()
            .map(|s| s.iter().map(engine_bar).collect())
            .collect(),
        start_ns,
    );
    let mut traces = Vec::new();
    while let Some(ev) = data.next_bar() {
        let in_window = ev.input == PRIMARY && !ev.warmup;
        if in_window {
            // The bar's fills happen before its close, so the engine hears of
            // them before it decides on the close.
            for e in venue.on_bar(&primary[&ev.bar.close_ns]) {
                traces.extend(cycle(&mut engine, e, models, &mut *venue, journal)?);
            }
        }
        let bar_event = Event::Bar {
            input: ev.input,
            bar: ev.bar,
            warmup: ev.warmup,
        };
        let decided = cycle(&mut engine, bar_event, models, &mut *venue, journal)?;
        if in_window {
            traces.extend(decided);
            venue.on_close_decided(&primary[&ev.bar.close_ns]);
        }
    }
    Ok(traces)
}

fn reason_key(m: &domain::strategy_graph::MissingReason) -> String {
    serde_json::to_value(m)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

fn cause_key(c: &Cause) -> &'static str {
    match c {
        Cause::BarClose => "bar_close",
        Cause::Fill { .. } => "fill",
        Cause::OrderClosed { .. } => "order_closed",
        Cause::Schedule => "schedule",
    }
}

fn manifest(
    plan: &Plan,
    policy: FillPolicy,
    limits: &GlobalRiskLimits,
    traces: &[DecisionTrace],
    gate_rejections: Vec<GateRejectionRecord>,
) -> EngineManifest {
    let mut holds = BTreeMap::new();
    let mut unknown = BTreeMap::new();
    let mut refusals = BTreeMap::new();
    let mut journal = Vec::new();
    let mut chain = Sha256::new();
    let mut decisions = 0u64;
    for t in traces {
        let digest = t.digest();
        chain.update(digest.as_bytes());
        if t.cause == Cause::BarClose {
            decisions += 1;
        }
        let mut held = 0u32;
        for outcome in t.rules.values() {
            match outcome {
                RuleOutcome::Held { reasons } => {
                    held += 1;
                    for m in reasons {
                        *holds.entry(reason_key(m)).or_insert(0) += 1;
                    }
                }
                RuleOutcome::Unknown { reason } => {
                    *unknown.entry(reason_key(reason)).or_insert(0) += 1;
                }
                _ => {}
            }
        }
        for r in &t.refusals {
            *refusals.entry(r.reason.clone()).or_insert(0) += 1;
        }
        if !t.actions.is_empty() || !t.refusals.is_empty() || held > 0 {
            journal.push(JournalEntry {
                at_ns: t.at_ns,
                tick: t.tick,
                cause: cause_key(&t.cause).to_string(),
                digest,
                actions: u32::try_from(t.actions.len()).unwrap_or(u32::MAX),
                refusals: u32::try_from(t.refusals.len()).unwrap_or(u32::MAX),
                holds: held,
            });
        }
    }
    let model_pins = plan
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Model(call) => Some(ModelPin {
                node: n.id.clone(),
                model: call.model.clone(),
                version: call.version,
            }),
            _ => None,
        })
        .collect();
    let mut risk_limits = BTreeMap::new();
    risk_limits.insert("max_position".into(), limits.max_position.to_string());
    risk_limits.insert(
        "max_daily_loss_usd".into(),
        limits.max_daily_loss_usd.to_string(),
    );
    risk_limits.insert(
        "max_orders_per_second".into(),
        limits.max_orders_per_second.to_string(),
    );
    risk_limits.insert(
        "max_orders_per_minute".into(),
        limits.max_orders_per_minute.to_string(),
    );
    risk_limits.insert("price_band_bps".into(), limits.price_band_bps.to_string());
    EngineManifest {
        engine: ENGINE_ID.into(),
        revision: plan.revision.clone(),
        fill_policy: match policy {
            FillPolicy::NextBarOpen => "next_bar_open".into(),
            FillPolicy::CurrentClose => "current_close".into(),
        },
        optimistic_fills: policy.is_optimistic(),
        venue: "simulated (paper venue core, ADR-0044)".into(),
        risk_limits,
        model_pins,
        folds: Vec::new(),
        decisions,
        holds_by_reason: holds,
        unknown_by_reason: unknown,
        planner_refusals: refusals,
        gate_rejections,
        journal_digest: hex::encode(chain.finalize()),
        journal,
    }
}

// ── the RunExecutor ──────────────────────────────────────────────────────────

/// Whether a stored definition is a v3 graph.
#[must_use]
pub fn is_v3(definition: &serde_json::Value) -> bool {
    definition.get("format").and_then(|f| f.as_str()) == Some(domain::strategy_graph::FORMAT)
}

/// The fill policy a config's fill model asks for.
///
/// # Errors
/// A fill model the native venue does not implement.
pub fn fill_policy(model: FillModel) -> Result<FillPolicy, String> {
    match model {
        // The native venue always resolves an ambiguous bar pessimistically
        // (stops before limits), so pessimistic-intrabar is next-bar-open with
        // that rule — which is what it already does.
        FillModel::NextBarOpen | FillModel::PessimisticIntrabar => Ok(FillPolicy::NextBarOpen),
        FillModel::CurrentClose => Ok(FillPolicy::CurrentClose),
        FillModel::LimitProb => Err(
            "the limit_prob fill model needs a queue model the native venue does not have".into(),
        ),
    }
}

/// The real v3 executor: the database and ClickHouse in, a [`RunResult`] out.
pub struct V3RunExecutor {
    handle: tokio::runtime::Handle,
    pg: sqlx::PgPool,
    ch_url: String,
    initial_balance: Decimal,
}

impl V3RunExecutor {
    #[must_use]
    pub fn new(
        handle: tokio::runtime::Handle,
        pg: sqlx::PgPool,
        ch_url: impl Into<String>,
    ) -> Self {
        Self {
            handle,
            pg,
            ch_url: ch_url.into(),
            initial_balance: Decimal::from(10_000),
        }
    }

    #[must_use]
    pub fn with_initial_balance(mut self, balance: Decimal) -> Self {
        self.initial_balance = balance;
        self
    }

    /// The stored definition: the exact revision when `strategy_version` names
    /// one, else the strategy's head.
    async fn definition(&self, cfg: &RunConfig) -> anyhow::Result<serde_json::Value> {
        let by_revision: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT definition_json FROM strategy_revision WHERE revision_hash = $1",
        )
        .bind(&cfg.strategy_version)
        .fetch_optional(&self.pg)
        .await?;
        if let Some((json,)) = by_revision {
            return Ok(json);
        }
        let head: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
        )
        .bind(&cfg.strategy_ref)
        .fetch_optional(&self.pg)
        .await?;
        head.map(|(j,)| j)
            .ok_or_else(|| anyhow::anyhow!("strategy '{}' not found", cfg.strategy_ref))
    }

    async fn load(&self, cfg: &RunConfig) -> Result<V3Run, (TerminalReason, String)> {
        let integrity = |e: String| (TerminalReason::IntegrityRejected, e);
        let json = self
            .definition(cfg)
            .await
            .map_err(|e| (TerminalReason::DependencyFailure, format!("{e:#}")))?;
        if !is_v3(&json) {
            return Err(integrity(format!(
                "'{}' is not a {} graph; v1 definitions run on SimRunExecutor",
                cfg.strategy_ref,
                domain::strategy_graph::FORMAT
            )));
        }
        let graph: StrategyGraph = serde_json::from_value(json)
            .map_err(|e| integrity(format!("graph does not parse: {e}")))?;
        let plan = compile_for_run(&graph, &cfg.params).map_err(integrity)?;
        check_resolution(&plan, cfg).map_err(integrity)?;

        let asset_class: AssetClass =
            serde_json::from_value(serde_json::Value::String(graph.asset_class.clone()))
                .map_err(|_| integrity(format!("unknown asset class '{}'", graph.asset_class)))?;
        let instrument = cfg.data_slice.universe_ref.clone();
        let meta = storage::postgres::instruments::fetch_by_id(&self.pg, &instrument)
            .await
            .map_err(|e| (TerminalReason::DependencyFailure, e.to_string()))?
            .ok_or_else(|| {
                (
                    TerminalReason::DataError,
                    format!(
                        "no instrument metadata for '{instrument}': the risk gate refuses an \
                         instrument it cannot size, here as live (ADR-0046)"
                    ),
                )
            })?;
        let fill = fill_policy(cfg.fill_model).map_err(integrity)?;

        let store = crate::store::BarStore::connect(&self.ch_url);
        let start_ns = cfg.data_slice.start.timestamp_nanos_opt().unwrap_or(0);
        let mut bars = Vec::with_capacity(plan.inputs.len());
        for (i, need) in plan.required_history() {
            let input = &plan.inputs[i];
            let id = match &input.instrument {
                domain::strategy_graph::InstrumentRef::Bound => instrument.clone(),
                domain::strategy_graph::InstrumentRef::Fixed { instrument } => instrument.clone(),
            };
            let lead = chrono::Duration::seconds(
                i64::from(need.saturating_add(1)) * i64::from(input.timeframe.seconds()),
            );
            let loaded = store
                .load_bars(
                    &id,
                    input.timeframe.timeframe(),
                    cfg.data_slice.start - lead,
                    cfg.data_slice.end,
                )
                .await
                .map_err(|e| {
                    (
                        TerminalReason::DataError,
                        format!("bar load for {id}: {e:#}"),
                    )
                })?;
            if loaded.is_empty() {
                return Err((
                    TerminalReason::DataError,
                    format!(
                        "no {} bars stored for {id} — initialise the asset first (runs never collect)",
                        input.timeframe.key()
                    ),
                ));
            }
            bars.push(loaded);
        }
        // A null-world run sees generated bars for the instrument it trades.
        if let Some(nw) = &cfg.null_world {
            bars[PRIMARY] = crate::sim_executor::apply_null_world(&bars[PRIMARY], nw);
        }
        let lot = meta.lot_size;
        Ok(V3Run {
            plan: Arc::new(plan),
            instrument: instrument.clone(),
            asset_class,
            bars,
            start_ns,
            spec: InstrumentSpec {
                instrument,
                lot_size: lot,
                tick_size: meta.tick_size,
                min_qty: lot,
                caps: strategy_engine::VenueCaps::all(),
            },
            instrument_limits: InstrumentLimits {
                tick_size: meta.tick_size,
                lot_size: lot,
            },
            limits: GlobalRiskLimits::default(),
            initial_balance: self.initial_balance,
            fill_policy: fill,
            strategy_id: cfg.strategy_ref.clone(),
            sims: SimulatorSet::realistic(),
        })
    }
}

/// Compile a graph with a run's parameters.
///
/// # Errors
/// Every diagnostic, joined.
pub fn compile_for_run(
    graph: &StrategyGraph,
    params: &crate::run::ParamMap,
) -> Result<Plan, String> {
    let overrides: domain::strategy_def::params::ParamValues =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    strategy_engine::compile(graph, &strategy_engine::StdRegistries::new(), &overrides).map_err(
        |diags| {
            diags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        },
    )
}

/// A run's eval resolution must be the plan's decision timeframe: the metrics
/// annualise by it.
fn check_resolution(plan: &Plan, cfg: &RunConfig) -> Result<(), String> {
    use crate::run::EvalResolution as E;
    let want = match cfg.data_slice.eval_resolution {
        E::Min1 => "1m",
        E::Min5 => "5m",
        E::Min15 => "15m",
        E::Hour1 => "1h",
        E::Day1 => "1d",
        E::Min10 | E::Min30 => "",
    };
    if want == plan.timeframe.key() {
        Ok(())
    } else {
        Err(format!(
            "the run's eval resolution ({:?}) is not the strategy's decision timeframe ({})",
            cfg.data_slice.eval_resolution,
            plan.timeframe.key()
        ))
    }
}

/// Map a v3 outcome into the run shape Set J judges.
#[must_use]
pub fn to_run_result(cfg: &RunConfig, o: V3Outcome, cost: ComputeCost) -> RunResult {
    let mut result = map_sim_result(cfg, o.equity, o.net_exposure, o.trades, cost, RUNNER_TAG);
    if o.manifest.optimistic_fills {
        result.integrity_flags.push(crate::run::Flag {
            code: "fills.optimistic".into(),
            detail: "orders filled at the decision bar's close (current_close), a price nobody \
                     could have traded at"
                .into(),
        });
    }
    result.manifest = Some(o.manifest);
    result
}

/// The document a backtest job stores for the run page: the run itself, with
/// the counts the existing readers of `backtest_jobs.result` look for.
#[must_use]
pub fn result_document(r: &RunResult) -> serde_json::Value {
    let mut doc = serde_json::to_value(r).unwrap_or_default();
    if let Some(o) = doc.as_object_mut() {
        let orders: u64 = r
            .manifest
            .as_ref()
            .map_or(0, |m| m.journal.iter().map(|j| u64::from(j.actions)).sum());
        o.insert("engine".into(), serde_json::json!(ENGINE_ID));
        o.insert(
            "trade_rows_harvested".into(),
            serde_json::json!(r.trades.len()),
        );
        o.insert("total_positions".into(), serde_json::json!(r.trades.len()));
        o.insert("total_orders".into(), serde_json::json!(orders));
    }
    doc
}

impl RunExecutor for V3RunExecutor {
    fn execute(&self, cfg: &RunConfig, ticket: &ledger::TrialTicket) -> RunResult {
        let started = Instant::now();
        tracing::debug!(trial_id = %ticket.trial_id(), run_id = cfg.run_id.as_str(), "v3 run dispatched");
        let loaded = tokio::task::block_in_place(|| self.handle.block_on(self.load(cfg)));
        let r = match loaded {
            Ok(r) => r,
            Err((reason, detail)) => return RunResult::failed(cfg, reason, detail, RUNNER_TAG),
        };
        let outcome = run(r, &mut strategy_engine::env::NoModels);
        let cost = ComputeCost {
            wall_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            cpu_ms: 0,
        };
        match outcome {
            Ok(o) => to_run_result(cfg, o, cost),
            Err(e) => RunResult::failed(
                cfg,
                TerminalReason::IntegrityRejected,
                e.to_string(),
                RUNNER_TAG,
            ),
        }
    }
}

/// The production executor for Experiments and Studies: a v3 strategy runs on
/// the one strategy engine, everything else on `market_simulator` as before
/// (Set U U-3.4; v1 keeps its path until Phase 8 retires it).
///
/// Which one a run is, is read from the stored document the run names — the
/// same lookup [`V3RunExecutor`] does — so a study over a v3 strategy reaches
/// the new runner without the study knowing there are two. A lookup that fails
/// goes to the v1 executor, which reports the missing strategy exactly as it
/// always has.
pub struct FormatRoutedExecutor {
    v1: crate::sim_executor::SimRunExecutor,
    v3: V3RunExecutor,
    handle: tokio::runtime::Handle,
}

impl FormatRoutedExecutor {
    #[must_use]
    pub fn new(
        handle: tokio::runtime::Handle,
        pg: sqlx::PgPool,
        ch_url: impl Into<String>,
    ) -> Self {
        let ch_url = ch_url.into();
        Self {
            v1: crate::sim_executor::SimRunExecutor::new(
                handle.clone(),
                pg.clone(),
                ch_url.clone(),
            ),
            v3: V3RunExecutor::new(handle.clone(), pg, ch_url),
            handle,
        }
    }
}

impl RunExecutor for FormatRoutedExecutor {
    fn execute(&self, cfg: &RunConfig, ticket: &ledger::TrialTicket) -> RunResult {
        let v3 = tokio::task::block_in_place(|| self.handle.block_on(self.v3.definition(cfg)))
            .is_ok_and(|json| is_v3(&json));
        if v3 {
            self.v3.execute(cfg, ticket)
        } else {
            self.v1.execute(cfg, ticket)
        }
    }
}

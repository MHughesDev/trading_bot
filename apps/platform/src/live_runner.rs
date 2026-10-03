//! `LiveRunner` — a v3 deployment on paper, through the one strategy engine
//! (Set U U-4.1, U-4.3, U-4.4, U-4.5; FEAT-007 §9.5; ADR-0039, ADR-0042,
//! ADR-0047).
//!
//! The same engine a backtest drives, with a different clock and a different
//! venue. Per deployment:
//!
//! - **Bars, not ticks.** The runner hears the pipeline's *completed* 1-minute
//!   bars and folds them into each declared input's timeframe; the engine is
//!   stepped when a bar of the decision timeframe closes, never per trade
//!   (ADR-0042). Hot-path stage 3 runs v1 strategies per tick and never sees a
//!   v3 deployment (`strategy_runtime::live`).
//! - **Indicators from bars alone.** The engine computes its own indicators
//!   from the bars it is given. Nothing injects a feature value — the defect
//!   that kept every indicator strategy on paper from ever firing (FEAT-007 §1
//!   #2).
//! - **The production order path.** Every order passes the process-wide risk
//!   context and the [`risk::RiskGate`], then the [`execution::ExecutionEngine`]
//!   — journaled before it is submitted, idempotent across a restart. Before
//!   each decision the runner asks the venue about its own working orders, so
//!   a resting order's fill reaches the engine before it decides again.
//! - **Models never block the path.** Requests in one tick are asked
//!   concurrently, each under its contract's timeout; a model that does not
//!   answer in time is `missing(model_timeout)` and its rule holds (U-4.3).
//! - **Every decision is journaled**, with the event and account it saw, the
//!   gate's verdict on each order and the reasons anything held (U-4.5). A
//!   journal write that fails halts the deployment: a decision that cannot be
//!   replayed is not one to keep making.
//! - **Checkpoint after every decision** (U-4.4): engine state plus the
//!   runner's working orders. A restart restores and resumes.
//!
//! This loop is the async twin of [`strategy_engine::cycle`]: the same steps in
//! the same order — event, model answers, decision, journal, act, feed
//! refusals back — with the venue and models awaited instead of called.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use domain::money::{Price, Size};
use domain::order::{OrderIntent, OrderState, OrderType, Side};
use execution::paper::PaperTradingEngine;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strategy_engine::decide::RuleOutcome;
use strategy_engine::plan::{InputIdx, NodeKind, PRIMARY};
use strategy_engine::planner::{OrderAction, OrderRole, PlannedOrder, PlannedStyle};
use strategy_engine::trace::Cause;
use strategy_engine::{
    Bar, Checkpoint, DecisionTrace, Engine, EngineError, Event, Fill, InstrumentSpec, ModelOutcome,
    ModelRequest, ModelResponse, OrderClosed, OrderClosedReason, Plan, PortfolioView, RestoreError,
    Step, WorkingOrder,
};
use strategy_runtime::live::V3Deployment;
use tracing::{error, info, warn};
use uuid::Uuid;

// ── ports the runner needs from the process ─────────────────────────────────

/// The shared live model-provider interface (`model-registry`, Set U U-5.3).
pub use model_registry::provider::LiveModels;

/// Until Phase 5 wires model serving, no model answers live: every request is
/// `unavailable`, and the engine holds rather than guessing.
pub struct NoLiveModels;

#[async_trait::async_trait]
impl LiveModels for NoLiveModels {
    async fn answer(&self, req: ModelRequest) -> ModelOutcome {
        ModelOutcome::Unavailable {
            detail: format!(
                "no live model serving yet for {}@{}",
                req.model, req.version
            ),
        }
    }
}

/// Where a deployment's journal and checkpoint live.
#[async_trait::async_trait]
pub trait DeploymentStore: Send + Sync {
    async fn append_journal(
        &self,
        record: storage::postgres::decision_journal::JournalRecord,
    ) -> Result<(), String>;
    async fn last_seq(&self, deployment_id: Uuid) -> Result<i64, String>;
    async fn save_checkpoint(
        &self,
        deployment_id: Uuid,
        revision: &str,
        tick: i64,
        state: serde_json::Value,
    ) -> Result<(), String>;
    async fn load_checkpoint(
        &self,
        deployment_id: Uuid,
    ) -> Result<Option<(String, serde_json::Value)>, String>;
}

/// The Postgres store (migrations 0142–0144).
pub struct PgDeploymentStore(pub sqlx::PgPool);

#[async_trait::async_trait]
impl DeploymentStore for PgDeploymentStore {
    async fn append_journal(
        &self,
        record: storage::postgres::decision_journal::JournalRecord,
    ) -> Result<(), String> {
        storage::postgres::decision_journal::append(&self.0, &record)
            .await
            .map_err(|e| e.to_string())
    }
    async fn last_seq(&self, deployment_id: Uuid) -> Result<i64, String> {
        storage::postgres::decision_journal::last_seq(&self.0, deployment_id)
            .await
            .map_err(|e| e.to_string())
    }
    async fn save_checkpoint(
        &self,
        deployment_id: Uuid,
        revision: &str,
        tick: i64,
        state: serde_json::Value,
    ) -> Result<(), String> {
        storage::postgres::deployments::save_checkpoint(
            &self.0,
            deployment_id,
            revision,
            tick,
            &state,
        )
        .await
        .map_err(|e| e.to_string())
    }
    async fn load_checkpoint(
        &self,
        deployment_id: Uuid,
    ) -> Result<Option<(String, serde_json::Value)>, String> {
        storage::postgres::deployments::load_checkpoint(&self.0, deployment_id)
            .await
            .map(|r| r.map(|c| (c.revision_hash, c.state)))
            .map_err(|e| e.to_string())
    }
}

/// Everything a runner shares with the rest of the process.
#[derive(Clone)]
pub struct LiveDeps {
    pub paper: Arc<PaperTradingEngine>,
    pub exec: Arc<execution::ExecutionEngine>,
    pub gate: Arc<risk::RiskGate>,
    pub models: Arc<dyn LiveModels>,
    pub store: Arc<dyn DeploymentStore>,
    /// The inference sidecar. When set, each deployment answers its models
    /// through a `GatewayModels` over the registry it compiled against, in place
    /// of `models` (Set U U-5.3).
    pub sidecar: Option<Arc<dyn model_registry::provider::V2Client>>,
    /// The LLM client for external LLM adapters, with its cost caps (U-5.6).
    pub llm: Option<(
        Arc<dyn model_registry::provider::llm::LlmClient>,
        model_registry::provider::llm::LlmCaps,
    )>,
}

// ── bars of a timeframe from bars of a minute ────────────────────────────────

/// Folds completed 1-minute bars into bars of one timeframe, closing on the
/// UTC-aligned boundary. A bar is emitted when its last minute arrives, or —
/// when that minute had no trades — when the first minute of the next bar does.
#[derive(Debug)]
struct Fold {
    period_ns: i64,
    current: Option<(i64, Bar)>,
}

impl Fold {
    fn new(seconds: u32) -> Self {
        Self {
            period_ns: i64::from(seconds) * 1_000_000_000,
            current: None,
        }
    }

    /// Fold one closed minute; returns the bars of this timeframe it closed.
    fn push(&mut self, m: Bar) -> Vec<Bar> {
        let mut out = Vec::new();
        let start = (m.close_ns - 1).div_euclid(self.period_ns) * self.period_ns;
        let end = start + self.period_ns;
        if let Some((s, _)) = self.current {
            if s != start {
                if let Some((_, b)) = self.current.take() {
                    out.push(b);
                }
            }
        }
        let merged = match self.current.take() {
            None => Bar { close_ns: end, ..m },
            Some((_, b)) => Bar {
                close_ns: end,
                open: b.open,
                high: b.high.max(m.high),
                low: b.low.min(m.low),
                close: m.close,
                volume: b.volume + m.volume,
            },
        };
        if m.close_ns >= end {
            out.push(merged);
        } else {
            self.current = Some((start, merged));
        }
        out
    }
}

// ── the runner ───────────────────────────────────────────────────────────────

/// One order this deployment placed.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LiveOrder {
    key: Uuid,
    broker_order_id: Option<String>,
    side: Side,
    qty: Decimal,
    filled: Decimal,
    reduce_only: bool,
    protective: bool,
    closed: bool,
}

/// What the runner checkpoints beside the engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RunnerState {
    engine: Checkpoint,
    orders: BTreeMap<String, LiveOrder>,
    bars_in_position: u32,
    high_water: Option<Decimal>,
    last_fill_price: Option<Decimal>,
}

/// What the risk gate (or the venue) said about one order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateVerdict {
    pub tag: String,
    /// `approved`, `rejected`, `no_context`, `venue_refused`.
    pub verdict: String,
    pub detail: Option<String>,
}

/// Why a runner stopped or could not start.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
    #[error("restore: {0}")]
    Restore(#[from] RestoreError),
    #[error("journal write failed — deployment halted: {0}")]
    Journal(String),
    #[error("checkpoint write failed — deployment halted: {0}")]
    Checkpoint(String),
    #[error("store: {0}")]
    Store(String),
}

/// One v3 deployment running on paper.
pub struct LiveRunner {
    d: Arc<V3Deployment>,
    plan: Arc<Plan>,
    asset_class: AssetClass,
    spec: InstrumentSpec,
    deps: LiveDeps,
    engine: Engine,
    folds: Vec<Fold>,
    orders: BTreeMap<String, LiveOrder>,
    seq: i64,
    bars_in_position: u32,
    high_water: Option<Decimal>,
    last_fill_price: Option<Decimal>,
    /// `on_event` calls — what U-4.1's "stepped once per closed bar" counts.
    steps: u64,
    halted: Option<String>,
    /// The close of the last minute folded, per instrument. A minute at or
    /// before it is one the runner already has — from warm-up, or a duplicate
    /// delivery — and is not folded twice.
    last_minute: BTreeMap<String, i64>,
}

/// Unix ns → UTC.
fn at(ns: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(ns)
}

fn sha(v: &serde_json::Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
}

impl LiveRunner {
    /// Start a deployment's runner: restore from its checkpoint when it has one,
    /// otherwise warm up from `history` (each input's stored bars, oldest first).
    ///
    /// # Errors
    /// A checkpoint from another revision, or a store that cannot be read.
    pub async fn start(
        d: Arc<V3Deployment>,
        plan: Arc<Plan>,
        asset_class: AssetClass,
        spec: InstrumentSpec,
        deps: LiveDeps,
        history: Vec<Vec<Bar>>,
    ) -> Result<Self, LiveError> {
        let folds = plan
            .inputs
            .iter()
            .map(|i| Fold::new(i.timeframe.seconds()))
            .collect();
        let seq = deps
            .store
            .last_seq(d.deployment_id)
            .await
            .map_err(LiveError::Store)?;
        let history_close: Vec<Option<i64>> = history
            .iter()
            .map(|bars| bars.iter().map(|b| b.close_ns).max())
            .collect();
        let flat: Vec<(InputIdx, Bar)> = history
            .into_iter()
            .enumerate()
            .flat_map(|(i, bars)| bars.into_iter().map(move |b| (i, b)))
            .collect();
        let saved = deps
            .store
            .load_checkpoint(d.deployment_id)
            .await
            .map_err(LiveError::Store)?;
        let mut runner = match saved {
            Some((revision, state)) => {
                if revision != plan.revision {
                    return Err(LiveError::Restore(RestoreError::PlanMismatch {
                        checkpoint: revision,
                        plan: plan.revision.clone(),
                    }));
                }
                let state: RunnerState = serde_json::from_value(state)
                    .map_err(|e| LiveError::Store(format!("checkpoint does not parse: {e}")))?;
                let engine = Engine::restore(Arc::clone(&plan), &state.engine, flat, &spec)?;
                info!(deployment = %d.deployment_id, tick = engine.tick(), "v3 deployment restored from its checkpoint");
                Self {
                    d,
                    plan,
                    asset_class,
                    spec,
                    deps,
                    engine,
                    folds,
                    orders: state.orders,
                    seq,
                    bars_in_position: state.bars_in_position,
                    high_water: state.high_water,
                    last_fill_price: state.last_fill_price,
                    steps: 0,
                    halted: None,
                    last_minute: BTreeMap::new(),
                }
            }
            None => {
                let mut engine = Engine::new(Arc::clone(&plan))?;
                let mut bars = flat;
                bars.sort_by_key(|(i, b)| (b.close_ns, *i == PRIMARY, *i));
                let view = PortfolioView::default();
                for (input, bar) in bars {
                    engine.on_event(
                        Event::Bar {
                            input,
                            bar,
                            warmup: true,
                        },
                        &view,
                        &spec,
                    )?;
                }
                Self {
                    d,
                    plan,
                    asset_class,
                    spec,
                    deps,
                    engine,
                    folds,
                    orders: BTreeMap::new(),
                    seq,
                    bars_in_position: 0,
                    high_water: None,
                    last_fill_price: None,
                    steps: 0,
                    halted: None,
                    last_minute: BTreeMap::new(),
                }
            }
        };
        runner.steps = 0;
        for (i, bars) in history_close.into_iter().enumerate() {
            if let Some(last) = bars {
                let instrument = runner.instrument_of(i);
                let e = runner.last_minute.entry(instrument).or_insert(last);
                *e = (*e).max(last);
            }
        }
        Ok(runner)
    }

    #[must_use]
    pub fn deployment_id(&self) -> Uuid {
        self.d.deployment_id
    }

    /// Engine steps taken since start — one per event, never one per tick.
    #[must_use]
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Why the deployment halted, if it did.
    #[must_use]
    pub fn halted(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    /// The instruments whose minute bars this deployment needs.
    #[must_use]
    pub fn instruments(&self) -> Vec<String> {
        let mut v: Vec<String> = (0..self.plan.inputs.len())
            .map(|i| self.instrument_of(i))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn instrument_of(&self, input: InputIdx) -> String {
        match &self.plan.inputs[input].instrument {
            domain::strategy_graph::InstrumentRef::Bound => self.d.instrument_id.clone(),
            domain::strategy_graph::InstrumentRef::Fixed { instrument } => instrument.clone(),
        }
    }

    /// One completed 1-minute bar of `instrument`. Returns every decision it
    /// caused.
    ///
    /// # Errors
    /// A failed journal or checkpoint write (the deployment halts), or an
    /// engine defect.
    pub async fn on_minute_bar(
        &mut self,
        instrument: &str,
        minute: Bar,
    ) -> Result<Vec<DecisionTrace>, LiveError> {
        if self.halted.is_some() {
            return Ok(Vec::new());
        }
        if self
            .last_minute
            .get(instrument)
            .is_some_and(|last| minute.close_ns <= *last)
        {
            return Ok(Vec::new());
        }
        self.last_minute
            .insert(instrument.to_owned(), minute.close_ns);
        let mut closed: Vec<(InputIdx, Bar)> = Vec::new();
        for i in 0..self.folds.len() {
            if self.instrument_of(i) == instrument {
                closed.extend(self.folds[i].push(minute).into_iter().map(|b| (i, b)));
            }
        }
        closed.sort_by_key(|(i, b)| (b.close_ns, *i == PRIMARY, *i));
        let mut decided = Vec::new();
        for (input, bar) in closed {
            if input == PRIMARY {
                // What happened to this strategy's working orders since it last
                // decided reaches the engine before it decides again.
                for e in self.sync_orders(bar.close_ns).await {
                    decided.extend(self.step(e).await?);
                }
                self.track_position(&bar);
            }
            decided.extend(
                self.step(Event::Bar {
                    input,
                    bar,
                    warmup: false,
                })
                .await?,
            );
        }
        Ok(decided)
    }

    /// Fold minutes that already elapsed in the current bucket of a longer
    /// timeframe, without deciding — so the first live hourly or daily bar is
    /// a whole bar, not the part of it after the runner started.
    pub fn prime_minute(&mut self, instrument: &str, minute: Bar) {
        for i in 0..self.folds.len() {
            if self.instrument_of(i) == instrument {
                // Primed minutes lie inside the open bucket; a bar this closes
                // was already in the warm-up history.
                let _ = self.folds[i].push(minute);
            }
        }
        let e = self
            .last_minute
            .entry(instrument.to_owned())
            .or_insert(minute.close_ns);
        *e = (*e).max(minute.close_ns);
    }

    /// The timeframe, in seconds, of each input, with its instrument.
    #[must_use]
    pub fn input_timeframes(&self) -> Vec<(String, u32)> {
        (0..self.plan.inputs.len())
            .map(|i| {
                (
                    self.instrument_of(i),
                    self.plan.inputs[i].timeframe.seconds(),
                )
            })
            .collect()
    }

    fn track_position(&mut self, bar: &Bar) {
        let pos = self.position();
        if pos.is_zero() {
            self.bars_in_position = 0;
            self.high_water = None;
        } else {
            self.bars_in_position += 1;
            let fav = Decimal::from_f64_retain(if pos.is_sign_positive() {
                bar.high
            } else {
                bar.low
            });
            if let Some(fav) = fav {
                self.high_water = Some(self.high_water.map_or(fav, |h| {
                    if pos.is_sign_positive() {
                        h.max(fav)
                    } else {
                        h.min(fav)
                    }
                }));
            }
        }
    }

    fn position(&self) -> Decimal {
        self.deps
            .paper
            .positions(self.asset_class)
            .iter()
            .find(|p| p.instrument_id == self.d.instrument_id)
            .map_or(Decimal::ZERO, |p| p.quantity)
    }

    fn view(&self) -> PortfolioView {
        let snap = self.deps.paper.snapshot(self.asset_class);
        let pos = snap
            .positions
            .iter()
            .find(|p| p.instrument_id == self.d.instrument_id);
        let unrealized: Decimal = snap.positions.iter().map(|p| p.unrealized_pnl).sum();
        let daily_loss = self.deps.paper.daily_loss_usd(self.asset_class);
        let position = pos.map_or(Decimal::ZERO, |p| p.quantity);
        let mark = self
            .deps
            .paper
            .mark(&self.d.instrument_id)
            .map(Price::inner);
        // A member of a selection-bound deployment sizes against its share of
        // the deployment's capital (U-6.4) — the same rule its portfolio
        // backtest applies; a hand-bound deployment against the account.
        let (equity, cash) = match &self.d.sleeve {
            Some(s) => {
                let c = s.capital();
                (c, c - position * mark.unwrap_or_default())
            }
            None => (snap.equity, snap.cash),
        };
        PortfolioView {
            position,
            avg_entry_price: pos.map(|p| p.average_entry_price),
            bars_in_position: self.bars_in_position,
            high_water: self.high_water,
            equity,
            cash,
            realized_pnl_today: -daily_loss - unrealized,
            mark,
            open_orders: self
                .orders
                .iter()
                .filter(|(_, o)| !o.closed)
                .map(|(tag, o)| WorkingOrder {
                    tag: tag.clone(),
                    side: o.side,
                    remaining: o.qty - o.filled,
                    reduce_only: o.reduce_only,
                    protective: o.protective,
                })
                .collect(),
            last_fill_price: self.last_fill_price,
        }
    }

    /// How this member leaves its selection, once its allocator has said so.
    #[must_use]
    pub fn retiring(&self) -> Option<domain::strategy_graph::deployment::OnRemoval> {
        self.d.sleeve.as_ref().and_then(|s| s.retiring())
    }

    /// Flat, with nothing working: a member letting its strategy exit is done.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.position().is_zero() && self.orders.values().all(|o| o.closed)
    }

    /// `close` for a member that left its selection: cancel what is working and
    /// flatten with a reduce-only market order — through this runner's own gate,
    /// like every order it sends. Returns the gate's verdicts.
    pub async fn flatten(&mut self, at_ns: i64) -> Vec<GateVerdict> {
        let mut actions: Vec<OrderAction> = self
            .orders
            .iter()
            .filter(|(_, o)| !o.closed)
            .map(|(tag, _)| OrderAction::Cancel { tag: tag.clone() })
            .collect();
        let pos = self.position();
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
        let (verdicts, _) = self.act(&actions, at_ns).await;
        verdicts
    }

    /// The async `cycle`: one event through the engine to completion.
    async fn step(&mut self, event: Event) -> Result<Vec<DecisionTrace>, LiveError> {
        let mut queue = std::collections::VecDeque::from([event]);
        let mut decided = Vec::new();
        while let Some(ev) = queue.pop_front() {
            if self.halted.is_some() {
                break;
            }
            let view = self.view();
            let inputs = serde_json::json!({ "event": ev, "view": view, "spec": self.spec });
            self.steps += 1;
            let mut step = self.engine.on_event(ev, &view, &self.spec)?;
            while let Step::NeedModels(requests) = step {
                let responses = self.ask_models(requests).await;
                step = self.engine.resume(responses)?;
            }
            let Step::Decided(trace) = step else { continue };
            let (verdicts, follow_on) = self.act(&trace.actions, trace.at_ns).await;
            self.journal(&trace, &inputs, &verdicts).await?;
            self.checkpoint().await?;
            queue.extend(follow_on);
            decided.push(*trace);
        }
        Ok(decided)
    }

    /// Ask every model of one tick concurrently, each under its contract's
    /// timeout. A model that does not answer in time is simply not answered;
    /// the engine records it as `missing(model_timeout)`.
    async fn ask_models(&self, requests: Vec<ModelRequest>) -> Vec<ModelResponse> {
        let timeouts: BTreeMap<String, u32> = self
            .plan
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Model(call) => Some((n.id.clone(), call.contract.timeout_ms)),
                _ => None,
            })
            .collect();
        let order: Vec<String> = requests.iter().map(|r| r.node.clone()).collect();
        // One batch per pinned version — a provider answers it in one call
        // (U-5.3) — and the batches concurrently, each under the tightest
        // timeout among its requests (U-4.3).
        let mut groups: BTreeMap<(String, u32), Vec<ModelRequest>> = BTreeMap::new();
        for r in requests {
            groups
                .entry((r.model.clone(), r.version))
                .or_default()
                .push(r);
        }
        let asks = groups.into_values().map(|batch| {
            let models = Arc::clone(&self.deps.models);
            let ms = batch
                .iter()
                .map(|r| timeouts.get(&r.node).copied().unwrap_or(1_000))
                .min()
                .unwrap_or(1_000);
            async move {
                let nodes: Vec<String> = batch.iter().map(|r| r.node.clone()).collect();
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(ms)),
                    models.answer_batch(batch),
                )
                .await
                {
                    Ok(outcomes) => nodes
                        .into_iter()
                        .zip(outcomes)
                        .map(|(node, outcome)| ModelResponse { node, outcome })
                        .collect(),
                    Err(_) => Vec::new(),
                }
            }
        });
        let mut answered: Vec<ModelResponse> = futures_util::future::join_all(asks)
            .await
            .into_iter()
            .flatten()
            .collect();
        // Deterministic: in the order the engine asked.
        answered.sort_by_key(|r| order.iter().position(|n| *n == r.node));
        answered
    }

    fn intent(&self, o: &PlannedOrder) -> OrderIntent {
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
            self.d.instrument_id.clone(),
            o.side,
            order_type,
            Size::from_decimal(o.qty),
            limit.map(Price::from_decimal),
            Some(self.d.deployment_id.to_string()),
        );
        // Deterministic per (deployment, tag): a restart that resends an order
        // resends the same key, and the execution journal returns the recorded
        // broker id instead of placing it twice.
        i.idempotency_key = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("{}/{}", self.d.deployment_id, o.tag).as_bytes(),
        );
        i.time_in_force = o.tif;
        i.stop_price = stop.map(Price::from_decimal);
        i.trail = trail.map(Price::from_decimal);
        i.reduce_only = o.reduce_only;
        i.post_only = o.post_only;
        i.oco_group.clone_from(&o.oco_group);
        i
    }

    /// Carry out a decision: gate, submit, cancel. Returns the gate's verdict
    /// on each order and the events the venue raised at once (fills of orders
    /// that filled on submission, refusals).
    async fn act(&mut self, actions: &[OrderAction], at_ns: i64) -> (Vec<GateVerdict>, Vec<Event>) {
        let mut verdicts = Vec::new();
        let mut events = Vec::new();
        for action in actions {
            match action {
                OrderAction::Submit(o) => {
                    let intent = self.intent(o);
                    let mut order = LiveOrder {
                        key: intent.idempotency_key,
                        broker_order_id: None,
                        side: o.side,
                        qty: o.qty,
                        filled: Decimal::ZERO,
                        reduce_only: o.reduce_only,
                        protective: o.role == OrderRole::Protective,
                        closed: false,
                    };
                    let refuse = |verdict: &str, detail: String| {
                        (
                            GateVerdict {
                                tag: o.tag.clone(),
                                verdict: verdict.into(),
                                detail: Some(detail),
                            },
                            Event::OrderClosed(OrderClosed {
                                tag: o.tag.clone(),
                                reason: OrderClosedReason::Rejected,
                                at_ns,
                            }),
                        )
                    };
                    let outcome = match risk::context::context_for(&intent) {
                        Err(e) => Err(refuse("no_context", e.to_string())),
                        Ok(ctx) => match self.deps.gate.check(intent, &ctx) {
                            Err(r) => Err(refuse("rejected", r.to_string())),
                            Ok(approved) => match self.deps.exec.submit(approved).await {
                                Err(e) => Err(refuse("venue_refused", e.to_string())),
                                Ok(r) => Ok(r),
                            },
                        },
                    };
                    match outcome {
                        Err((v, e)) => {
                            warn!(deployment = %self.d.deployment_id, tag = %o.tag, verdict = %v.verdict, detail = ?v.detail, "order refused");
                            order.closed = true;
                            verdicts.push(v);
                            events.push(e);
                        }
                        Ok(submitted) => {
                            order.broker_order_id = Some(submitted.broker_order_id);
                            verdicts.push(GateVerdict {
                                tag: o.tag.clone(),
                                verdict: "approved".into(),
                                detail: None,
                            });
                            if let Some(f) = submitted.fill {
                                order.filled = f.cumulative_filled;
                                order.closed = order.filled >= order.qty;
                                self.last_fill_price = Some(f.fill_price.inner());
                                events.push(Event::Fill(Fill {
                                    tag: o.tag.clone(),
                                    side: o.side,
                                    qty: f.filled_size,
                                    price: f.fill_price.inner(),
                                    at_ns,
                                }));
                            }
                        }
                    }
                    self.orders.insert(o.tag.clone(), order);
                }
                OrderAction::Cancel { tag } => {
                    let Some(order) = self.orders.get_mut(tag) else {
                        continue;
                    };
                    if order.closed {
                        continue;
                    }
                    if let Some(id) = &order.broker_order_id {
                        if let Err(e) = self.deps.exec.cancel(id).await {
                            warn!(deployment = %self.d.deployment_id, tag, error = %e, "cancel failed");
                        }
                    }
                    order.closed = true;
                    events.push(Event::OrderClosed(OrderClosed {
                        tag: tag.clone(),
                        reason: OrderClosedReason::Cancelled,
                        at_ns,
                    }));
                }
            }
        }
        (verdicts, events)
    }

    /// Ask the venue about every working order; report fills and closures.
    async fn sync_orders(&mut self, at_ns: i64) -> Vec<Event> {
        let mut events = Vec::new();
        let open: Vec<(String, LiveOrder)> = self
            .orders
            .iter()
            .filter(|(_, o)| !o.closed && o.broker_order_id.is_some())
            .map(|(t, o)| (t.clone(), o.clone()))
            .collect();
        for (tag, mut o) in open {
            let Some(broker_id) = o.broker_order_id.clone() else {
                continue;
            };
            let synced = match self
                .deps
                .exec
                .sync_order(o.key, &broker_id, o.filled, o.side)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!(deployment = %self.d.deployment_id, tag, error = %e, "order sync failed; will retry next bar");
                    continue;
                }
            };
            if let Some(f) = synced.fill {
                o.filled += f.filled_qty;
                self.last_fill_price = Some(f.fill_price.inner());
                events.push(Event::Fill(Fill {
                    tag: tag.clone(),
                    side: o.side,
                    qty: f.filled_qty,
                    price: f.fill_price.inner(),
                    at_ns,
                }));
            }
            match synced.new_state {
                OrderState::Filled => o.closed = true,
                OrderState::Cancelled | OrderState::Rejected => {
                    o.closed = true;
                    events.push(Event::OrderClosed(OrderClosed {
                        tag: tag.clone(),
                        reason: if synced.new_state == OrderState::Cancelled {
                            OrderClosedReason::Cancelled
                        } else {
                            OrderClosedReason::Rejected
                        },
                        at_ns,
                    }));
                }
                _ => {}
            }
            self.orders.insert(tag, o);
        }
        events
    }

    async fn journal(
        &mut self,
        trace: &DecisionTrace,
        inputs: &serde_json::Value,
        verdicts: &[GateVerdict],
    ) -> Result<(), LiveError> {
        let mut missing: BTreeMap<String, u64> = BTreeMap::new();
        for outcome in trace.rules.values() {
            let reasons: Vec<&domain::strategy_graph::MissingReason> = match outcome {
                RuleOutcome::Held { reasons } => reasons.iter().collect(),
                RuleOutcome::Unknown { reason } => vec![reason],
                _ => Vec::new(),
            };
            for r in reasons {
                let key = serde_json::to_value(r)
                    .ok()
                    .and_then(|v| v.get("reason").and_then(|x| x.as_str()).map(str::to_string))
                    .unwrap_or_else(|| "unknown".into());
                *missing.entry(key).or_insert(0) += 1;
            }
        }
        self.seq += 1;
        let record = storage::postgres::decision_journal::JournalRecord {
            deployment_id: self.d.deployment_id,
            seq: self.seq,
            tick: i64::try_from(trace.tick).unwrap_or(i64::MAX),
            decided_at: at(trace.at_ns),
            cause: match trace.cause {
                Cause::BarClose => "bar_close",
                Cause::Fill { .. } => "fill",
                Cause::OrderClosed { .. } => "order_closed",
                Cause::Schedule => "schedule",
            }
            .into(),
            digest: trace.digest(),
            input_digest: sha(inputs),
            inputs: inputs.clone(),
            trace: serde_json::to_value(trace).unwrap_or_default(),
            gate_verdicts: serde_json::to_value(verdicts).unwrap_or_default(),
            missing_reasons: serde_json::to_value(&missing).unwrap_or_default(),
        };
        if let Err(e) = self.deps.store.append_journal(record).await {
            error!(deployment = %self.d.deployment_id, error = %e, "decision journal write failed — halting the deployment");
            self.halted = Some(format!("journal write failed: {e}"));
            return Err(LiveError::Journal(e));
        }
        Ok(())
    }

    async fn checkpoint(&mut self) -> Result<(), LiveError> {
        let engine = self.engine.checkpoint()?;
        let tick = i64::try_from(engine.tick).unwrap_or(i64::MAX);
        let state = RunnerState {
            engine,
            orders: self.orders.clone(),
            bars_in_position: self.bars_in_position,
            high_water: self.high_water,
            last_fill_price: self.last_fill_price,
        };
        let value =
            serde_json::to_value(&state).map_err(|e| LiveError::Checkpoint(e.to_string()))?;
        if let Err(e) = self
            .deps
            .store
            .save_checkpoint(self.d.deployment_id, &self.plan.revision, tick, value)
            .await
        {
            error!(deployment = %self.d.deployment_id, error = %e, "checkpoint write failed — halting the deployment");
            self.halted = Some(format!("checkpoint write failed: {e}"));
            return Err(LiveError::Checkpoint(e));
        }
        Ok(())
    }
}

/// A completed minute from the pipeline, as the engine reads it.
#[must_use]
pub fn minute_bar(b: &backtest::CollectedBar) -> Option<Bar> {
    let f = |s: &str| s.parse::<Decimal>().ok().and_then(|d| d.to_f64());
    Some(Bar {
        close_ns: b.available_time.timestamp_nanos_opt()?,
        open: f(&b.open)?,
        high: f(&b.high)?,
        low: f(&b.low)?,
        close: f(&b.close)?,
        volume: f(&b.volume)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Instant;

    use domain::model_def::contract::{ContractOutput, CostClass, IoContract, PitSpec};
    use domain::strategy_def::params::ParamValues;
    use domain::strategy_graph::{MissingReason, StrategyGraph, ValueType};
    use risk::{GlobalRiskLimits, KillSwitch, RiskGate};
    use storage::postgres::decision_journal::JournalRecord;
    use strategy_engine::{ResolvedModel, StdRegistries, VenueCaps};

    use crate::hot_path::Minute1Aggregator;
    use crate::test_support::{env, ready_instrument};

    // ── fixtures ─────────────────────────────────────────────────────────────

    #[derive(Default)]
    struct MemoryStore {
        journal: Mutex<Vec<JournalRecord>>,
        checkpoints: Mutex<BTreeMap<Uuid, (String, serde_json::Value)>>,
        fail_journal: bool,
    }

    #[async_trait::async_trait]
    impl DeploymentStore for MemoryStore {
        async fn append_journal(&self, record: JournalRecord) -> Result<(), String> {
            if self.fail_journal {
                return Err("disk full".into());
            }
            self.journal.lock().unwrap().push(record);
            Ok(())
        }
        async fn last_seq(&self, id: Uuid) -> Result<i64, String> {
            Ok(self
                .journal
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.deployment_id == id)
                .map(|r| r.seq)
                .max()
                .unwrap_or(0))
        }
        async fn save_checkpoint(
            &self,
            id: Uuid,
            revision: &str,
            _tick: i64,
            state: serde_json::Value,
        ) -> Result<(), String> {
            self.checkpoints
                .lock()
                .unwrap()
                .insert(id, (revision.to_owned(), state));
            Ok(())
        }
        async fn load_checkpoint(
            &self,
            id: Uuid,
        ) -> Result<Option<(String, serde_json::Value)>, String> {
            Ok(self.checkpoints.lock().unwrap().get(&id).cloned())
        }
    }

    struct SlowModels {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl LiveModels for SlowModels {
        async fn answer(&self, _req: ModelRequest) -> ModelOutcome {
            tokio::time::sleep(self.delay).await;
            ModelOutcome::Answered {
                outputs: [(
                    "f".to_string(),
                    domain::strategy_graph::Value::Num(1.0.try_into().unwrap()),
                )]
                .into_iter()
                .collect(),
            }
        }
    }

    fn deps(store: Arc<dyn DeploymentStore>, models: Arc<dyn LiveModels>) -> LiveDeps {
        let limits = GlobalRiskLimits {
            max_orders_per_second: 10_000,
            max_orders_per_minute: 100_000,
            ..GlobalRiskLimits::default()
        };
        LiveDeps {
            paper: Arc::clone(&env().paper),
            exec: Arc::new(execution::ExecutionEngine::new(Arc::new(
                env().paper.multi_asset_broker(),
            ))),
            gate: Arc::new(RiskGate::new(limits, Arc::new(KillSwitch::new(false)))),
            models,
            store,
            sidecar: None,
            llm: None,
        }
    }

    fn graph(v: serde_json::Value) -> StrategyGraph {
        serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}\n{v:#}"))
    }

    /// The acceptance strategy: `ema(close, 7) > ema(close, 21)`, on minutes.
    fn ema_7_21() -> StrategyGraph {
        graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "ema_7_21",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [
            {"id":"f","op":"ema","args":{"x":"close","n":7}},
            {"id":"s","op":"ema","args":{"x":"close","n":21}},
            {"id":"up","op":"gt","args":{"a":"f","b":"s"}},
            {"id":"dn","op":"lt","args":{"a":"f","b":"s"}}
          ],
          "decide": {"rules": [
            {"when":"up","then":{"kind":"target","size":{"mode":"units","x":0.5}}},
            {"when":"dn","then":{"kind":"target","size":{"mode":"units","x":0}}}
          ]}
        }))
    }

    fn deployment(instrument: &str, g: &StrategyGraph) -> Arc<V3Deployment> {
        Arc::new(V3Deployment {
            deployment_id: Uuid::new_v4(),
            user_id: "u-live".into(),
            instrument_id: instrument.into(),
            revision: g.revision().unwrap().as_str().to_string(),
            graph: g.clone(),
            starting_capital: Decimal::from(10_000),
            sleeve: None,
        })
    }

    fn spec(instrument: &str) -> InstrumentSpec {
        InstrumentSpec {
            instrument: instrument.into(),
            lot_size: Decimal::new(1, 8),
            tick_size: Decimal::new(1, 1),
            min_qty: Decimal::new(1, 8),
            caps: VenueCaps::all(),
        }
    }

    fn plan_of(g: &StrategyGraph, reg: &StdRegistries) -> Arc<Plan> {
        Arc::new(
            strategy_engine::compile(g, reg, &ParamValues::new())
                .unwrap_or_else(|d| panic!("{d:#?}")),
        )
    }

    /// A price path that falls, then rises, then falls: EMA 7 crosses 21 both ways.
    fn price(minute: usize) -> f64 {
        let x = minute as f64;
        100.0 + 6.0 * (x / 18.0).sin() + 0.05 * (x * 7.0).sin()
    }

    const T0: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

    /// Trades as the socket delivers them — five a minute — through the
    /// pipeline's own minute aggregator, marking the paper book as stage 2 does.
    /// Returns the completed minutes, and how many trades produced them.
    fn trades_to_minutes(instrument: &str, minutes: usize) -> (Vec<Bar>, usize) {
        let mut agg = Minute1Aggregator::new();
        let mut out = Vec::new();
        let mut trades = 0;
        for m in 0..minutes {
            for k in 0..5 {
                let p = price(m) + 0.01 * f64::from(k);
                let p_dec = Decimal::from_f64_retain((p * 100.0).round() / 100.0).unwrap();
                env().paper.on_mark(instrument, Price::from_decimal(p_dec));
                trades += 1;
                let ts = T0 + i64::try_from(m).unwrap() * 60 + i64::from(k) * 10;
                if let Some(done) = agg.on_trade(p_dec, Decimal::ONE, ts) {
                    out.push(minute_bar(&done).unwrap());
                }
            }
        }
        (out, trades)
    }

    fn submits(traces: &[DecisionTrace]) -> Vec<String> {
        traces
            .iter()
            .flat_map(|t| t.actions.iter())
            .filter_map(|a| match a {
                OrderAction::Submit(o) => Some(format!("{} {:?} {}", o.tag, o.side, o.qty)),
                OrderAction::Cancel { .. } => None,
            })
            .collect()
    }

    // ── U-4.1 ────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn an_ema_strategy_fires_on_paper_from_live_bars_alone() {
        let inst = "LIVE-EMA-USD";
        ready_instrument(inst, "100");
        let g = ema_7_21();
        let store = Arc::new(MemoryStore::default());
        let mut r = LiveRunner::start(
            deployment(inst, &g),
            plan_of(&g, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(store.clone(), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();

        let (minutes, trades) = trades_to_minutes(inst, 120);
        let mut decided = Vec::new();
        for m in &minutes {
            decided.extend(r.on_minute_bar(inst, *m).await.unwrap());
        }

        // It fired, and it filled on paper — with no feature value injected by
        // anything: the engine computed both EMAs from the bars.
        assert!(!submits(&decided).is_empty(), "the strategy placed orders");
        assert!(
            env()
                .paper
                .positions(AssetClass::CryptoSpotCex)
                .iter()
                .any(|p| p.instrument_id == inst)
                || decided
                    .iter()
                    .any(|t| matches!(t.cause, Cause::Fill { .. })),
            "orders reached the paper venue and filled"
        );
        // Stepped per closed bar, never per tick: one bar-close decision per
        // completed minute, and engine steps = bar events + fill/close events,
        // far below the trade count.
        let bar_closes = decided
            .iter()
            .filter(|t| t.cause == Cause::BarClose)
            .count();
        assert_eq!(bar_closes, minutes.len());
        // Every other step is an order event — a fill or a closure — at most two
        // per order placed. A fill with nothing to arm steps the engine and
        // decides nothing, so steps can exceed decisions; never by a tick.
        let orders = submits(&decided).len() as u64;
        assert!(r.steps() >= minutes.len() as u64);
        assert!(
            r.steps() <= minutes.len() as u64 + 2 * orders,
            "{} steps, {} bars, {orders} orders",
            r.steps(),
            minutes.len()
        );
        assert!(
            r.steps() < (trades / 3) as u64,
            "{} steps for {trades} trades",
            r.steps()
        );

        // U-4.5: exactly one journal record per decision, numbered without gaps.
        let journal = store.journal.lock().unwrap();
        assert_eq!(journal.len(), decided.len());
        assert!(journal
            .iter()
            .enumerate()
            .all(|(i, rec)| rec.seq == i as i64 + 1));
        assert!(journal
            .iter()
            .any(|rec| rec.gate_verdicts.as_array().is_some_and(|v| !v.is_empty())));
        assert!(journal
            .iter()
            .all(|rec| rec.inputs.get("view").is_some() && rec.inputs.get("event").is_some()));
    }

    // ── U-4.3 ────────────────────────────────────────────────────────────────

    fn contract(timeout_ms: u32) -> IoContract {
        IoContract {
            inputs: vec![],
            outputs: vec![ContractOutput {
                name: "f".into(),
                value_type: ValueType::Num,
            }],
            horizon: Some("1m".into()),
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms,
        }
    }

    fn model_registry(refs: &[(&str, u32)]) -> StdRegistries {
        refs.iter().fold(StdRegistries::new(), |reg, (r, t)| {
            reg.with_model(
                r,
                ResolvedModel {
                    model: r.split('@').next().unwrap().into(),
                    version: 1,
                    contract: contract(*t),
                },
            )
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_model_past_its_timeout_holds_on_time() {
        let inst = "LIVE-SLOW-USD";
        ready_instrument(inst, "100");
        let g = graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "slow_model",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [
            {"id":"m","op":"model","args":{"ref":{"str":"slow@1"}}},
            {"id":"go","op":"ge","args":{"a":"close","b":0}}
          ],
          "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":"m.f"}}}]}
        }));
        let store = Arc::new(MemoryStore::default());
        let mut r = LiveRunner::start(
            deployment(inst, &g),
            plan_of(&g, &model_registry(&[("slow@1", 50)])),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(
                store.clone(),
                Arc::new(SlowModels {
                    delay: Duration::from_millis(400),
                }),
            ),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let started = Instant::now();
        let decided = r
            .on_minute_bar(
                inst,
                Bar {
                    close_ns: T0 * 1_000_000_000 + 60_000_000_000,
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.0,
                    volume: 1.0,
                },
            )
            .await
            .unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(300),
            "the hot path waited {took:?} on a 50 ms model"
        );
        let RuleOutcome::Held { reasons } = &decided[0].rules[&0] else {
            panic!("expected a hold, got {:?}", decided[0].rules[&0]);
        };
        assert!(
            matches!(
                reasons[0],
                MissingReason::ModelTimeout { timeout_ms: 50, .. }
            ),
            "{reasons:?}"
        );
        assert!(decided[0].actions.is_empty());
        let rec = &store.journal.lock().unwrap()[0];
        assert_eq!(rec.missing_reasons["model_timeout"], 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_models_in_one_layer_are_asked_at_once() {
        let inst = "LIVE-PAR-USD";
        ready_instrument(inst, "100");
        let g = graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "two_models",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [
            {"id":"a","op":"model","args":{"ref":{"str":"a@1"}}},
            {"id":"b","op":"model","args":{"ref":{"str":"b@1"}}},
            {"id":"s","op":"add","args":{"a":"a.f","b":"b.f"}},
            {"id":"go","op":"ge","args":{"a":"s","b":0}}
          ],
          "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
        }));
        let mut r = LiveRunner::start(
            deployment(inst, &g),
            plan_of(&g, &model_registry(&[("a@1", 2_000), ("b@1", 2_000)])),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(
                Arc::new(MemoryStore::default()),
                Arc::new(SlowModels {
                    delay: Duration::from_millis(200),
                }),
            ),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let started = Instant::now();
        let decided = r
            .on_minute_bar(
                inst,
                Bar {
                    close_ns: T0 * 1_000_000_000 + 60_000_000_000,
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.0,
                    volume: 1.0,
                },
            )
            .await
            .unwrap();
        let took = started.elapsed();
        assert_eq!(decided[0].model_responses.len(), 2, "both answered");
        assert!(took >= Duration::from_millis(200));
        assert!(
            took < Duration::from_millis(380),
            "two 200 ms models took {took:?}: asked one after the other"
        );
    }

    // ── U-4.4 ────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_mid_position_resumes_without_a_duplicate_entry() {
        let g = ema_7_21();
        let plan = plan_of(&g, &StdRegistries::new());

        // The reference: one runner that never stops.
        let reference = "LIVE-REF-USD";
        ready_instrument(reference, "100");
        let (minutes, _) = trades_to_minutes(reference, 140);
        let mut whole = LiveRunner::start(
            deployment(reference, &g),
            Arc::clone(&plan),
            AssetClass::CryptoSpotCex,
            spec(reference),
            deps(Arc::new(MemoryStore::default()), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let mut expected = Vec::new();
        for m in &minutes {
            expected.extend(whole.on_minute_bar(reference, *m).await.unwrap());
        }

        // The same strategy on its own instrument, killed mid-position.
        let inst = "LIVE-RESTART-USD";
        ready_instrument(inst, "100");
        let _ = trades_to_minutes(inst, 140); // the same marks on this book
        let d = deployment(inst, &g);
        let store = Arc::new(MemoryStore::default());
        let mut first = LiveRunner::start(
            Arc::clone(&d),
            Arc::clone(&plan),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(store.clone(), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let mut got = Vec::new();
        let mut k = 0;
        for (i, m) in minutes.iter().enumerate() {
            got.extend(first.on_minute_bar(inst, *m).await.unwrap());
            // Kill it once it holds a position and a few more bars have passed.
            if !first.position().is_zero() && i > 40 {
                k = i + 1;
                break;
            }
        }
        assert!(
            k > 0 && !first.position().is_zero(),
            "the kill happens mid-position"
        );
        drop(first);

        // Restart: the stored minutes so far, and the checkpoint.
        let history = vec![minutes[..k].to_vec()];
        let mut second = LiveRunner::start(
            Arc::clone(&d),
            Arc::clone(&plan),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(store.clone(), Arc::new(NoLiveModels)),
            history,
        )
        .await
        .unwrap();
        for m in &minutes[k..] {
            got.extend(second.on_minute_bar(inst, *m).await.unwrap());
        }
        // The same orders, tag for tag, as the runner that never stopped — so
        // no duplicate entry after the restart, and no missed exit either.
        assert_eq!(submits(&got), submits(&expected));
        let seqs: Vec<i64> = store
            .journal
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.seq)
            .collect();
        assert_eq!(
            seqs,
            (1..=seqs.len() as i64).collect::<Vec<_>>(),
            "journal numbering continues"
        );

        // A checkpoint from another plan is refused.
        let other = graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "ema_7_21",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
          "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
        }));
        let err = LiveRunner::start(
            d,
            plan_of(&other, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(store, Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .err()
        .expect("refused");
        assert!(
            matches!(err, LiveError::Restore(RestoreError::PlanMismatch { .. })),
            "{err}"
        );
    }

    // ── U-4.5 ────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_journal_that_cannot_be_written_halts_the_deployment() {
        let inst = "LIVE-NOJOURNAL-USD";
        ready_instrument(inst, "100");
        let g = graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "always",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
          "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
        }));
        let store = Arc::new(MemoryStore {
            fail_journal: true,
            ..MemoryStore::default()
        });
        let mut r = LiveRunner::start(
            deployment(inst, &g),
            plan_of(&g, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(store, Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let bar = |i: i64| Bar {
            close_ns: (T0 + 60 * i) * 1_000_000_000,
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 1.0,
        };
        let err = r
            .on_minute_bar(inst, bar(1))
            .await
            .expect_err("surfaced, not swallowed");
        assert!(matches!(err, LiveError::Journal(_)), "{err}");
        assert!(r.halted().is_some());
        assert!(
            r.on_minute_bar(inst, bar(2)).await.unwrap().is_empty(),
            "a halted deployment decides nothing"
        );
    }

    // ── U-4.6 ────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_withdrawn_deployment_stops_within_one_decision_tick() {
        let inst = "LIVE-STOP-USD";
        ready_instrument(inst, "100");
        let g = ema_7_21();
        let d = deployment(inst, &g);
        let registry = Arc::new(strategy_runtime::live::LiveStrategyRegistry::new());
        registry.register_v3((*d).clone()).unwrap();
        let runner = LiveRunner::start(
            Arc::clone(&d),
            plan_of(&g, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(Arc::new(MemoryStore::default()), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(64);
        let reg = Arc::clone(&registry);
        let task =
            tokio::spawn(
                async move { crate::live_deployments::drive(runner, &mut rx, &reg).await },
            );
        let minute = |i: i64| crate::live_bars::MinuteBar {
            instrument_id: inst.into(),
            bar: backtest::CollectedBar {
                available_time: DateTime::from_timestamp(T0 + 60 * i, 0).unwrap(),
                sequence: 0,
                open: "100".into(),
                high: "100".into(),
                low: "100".into(),
                close: "100".into(),
                volume: "1".into(),
                trade_count: 1,
            },
        };
        tx.send(minute(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!task.is_finished(), "still running while declared");
        assert!(registry.deregister_v3(d.deployment_id));
        tx.send(minute(2)).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(2), task).await;
        assert!(
            matches!(ended, Ok(Ok(None))),
            "stopped at the next bar, withdrawn not halted: {ended:?}"
        );
    }

    // ── U-6.4: a member of a selection-bound deployment ──────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_member_sizes_against_its_sleeve_and_a_close_flattens_it_through_the_gate() {
        let inst = "LIVE-SLEEVE-USD";
        ready_instrument(inst, "100");
        let g = graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "half_in",
          "asset_class": "crypto_spot_cex", "timeframe": "1m",
          "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
          "decide": {"rules": [
            {"when":"go","then":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}}}
          ]}
        }));
        let sleeve = Arc::new(strategy_runtime::live::SleeveHandle::new(
            Uuid::new_v4(),
            Decimal::from(2_000),
        ));
        let mut d = (*deployment(inst, &g)).clone();
        d.sleeve = Some(Arc::clone(&sleeve));
        let mut r = LiveRunner::start(
            Arc::new(d),
            plan_of(&g, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(Arc::new(MemoryStore::default()), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        let bar = Bar {
            close_ns: (T0 + 60) * 1_000_000_000,
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 1.0,
        };
        let decided = r.on_minute_bar(inst, bar).await.unwrap();
        // Half of its 2,000 sleeve at 100 is 10 units — not half the account.
        assert_eq!(
            submits(&decided)
                .first()
                .map(|s| s.ends_with(" Buy 10") || s.ends_with(" Buy 10.00000000")),
            Some(true),
            "{:?}",
            submits(&decided)
        );
        assert_eq!(r.position(), Decimal::from(10));

        assert!(
            crate::live_deployments::retire(&mut r).await.is_none(),
            "not retiring yet"
        );
        sleeve.retire(domain::strategy_graph::deployment::OnRemoval::Close);
        let reason = crate::live_deployments::retire(&mut r)
            .await
            .expect("stops");
        assert_eq!(reason, "left its selection: flattened");
        assert!(
            r.position().is_zero(),
            "flattened through the runner's own gate"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_member_letting_its_strategy_exit_stops_only_when_flat() {
        let inst = "LIVE-EXIT-USD";
        ready_instrument(inst, "100");
        let g = ema_7_21();
        let sleeve = Arc::new(strategy_runtime::live::SleeveHandle::new(
            Uuid::new_v4(),
            Decimal::from(5_000),
        ));
        let mut d = (*deployment(inst, &g)).clone();
        d.sleeve = Some(Arc::clone(&sleeve));
        let mut r = LiveRunner::start(
            Arc::new(d),
            plan_of(&g, &StdRegistries::new()),
            AssetClass::CryptoSpotCex,
            spec(inst),
            deps(Arc::new(MemoryStore::default()), Arc::new(NoLiveModels)),
            vec![Vec::new()],
        )
        .await
        .unwrap();
        // Long enough for a full cycle of the price path: in, then out again.
        let (minutes, _) = trades_to_minutes(inst, 360);
        let mut bars = minutes.iter();
        // Trade until it holds something, then drop it from the selection.
        for m in bars.by_ref() {
            r.on_minute_bar(inst, *m).await.unwrap();
            if !r.position().is_zero() {
                break;
            }
        }
        assert!(!r.position().is_zero(), "the strategy entered");
        sleeve.retire(domain::strategy_graph::deployment::OnRemoval::LetStrategyExit);
        let mut held_while_long = 0;
        let mut stopped = None;
        for m in bars {
            match crate::live_deployments::retire(&mut r).await {
                None => held_while_long += 1,
                Some(reason) => {
                    stopped = Some(reason);
                    break;
                }
            }
            r.on_minute_bar(inst, *m).await.unwrap();
        }
        assert!(
            held_while_long > 1,
            "it kept deciding while it held a position"
        );
        assert_eq!(
            stopped.as_deref(),
            Some("left its selection: its strategy exited")
        );
        assert!(r.is_idle());
    }
}

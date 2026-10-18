//! In-process hot-path pipeline: socket-reader → bar-builder → strategy-eval → risk/exec.
//!
//! Each stage communicates via a bounded lock-free SPSC ring (rtrb).  JetStream
//! never appears on this path — it receives events via the async tee task.

use std::sync::Arc;

use backtest::CollectedBar;
use collectors::CollectorError;
use domain::{instrument::AssetClass, EventEnvelope, OrderIntent};
use execution::paper::PaperTradingEngine;
use rust_decimal::Decimal;
use strategy_runtime::world::WorldEvent;
use tracing::{info, warn};

use crate::bar_persist::PersistBar;

/// A normalized trade tick from the socket reader.
pub type RawTick = EventEnvelope;

/// Where a pipeline sends its completed 1-minute bars, plus the venue metadata
/// stamped onto each persisted bar.  One value per pipeline (the source venue is
/// fixed for the life of the pipeline).
#[derive(Clone)]
pub struct BarSink {
    pub tx: tokio::sync::mpsc::UnboundedSender<PersistBar>,
    pub venue_id: String,
    pub source: String,
    pub trust_tier: String,
}

/// Keeps the spawned pipeline tasks alive.  Drop to stop the pipeline.
pub struct PipelineHandle {
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Wire the full in-process pipeline for a single instrument and return a handle.
///
/// Stages:
///   1. socket-reader  →  ring_raw   (cap: 4 096)
///   2. bar-builder    →  ring_world (cap: 1 024)
///   3. strategy-eval  →  ring_intent (cap: 256)
///   4. risk/exec      →  broker
#[allow(clippy::too_many_arguments)]
pub fn spawn_pipeline(
    symbol: String,
    instrument_id: String,
    asset_class: AssetClass,
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
    // Paper engine fed by stage 2: every tick updates the per-instrument mark
    // board and fills any resting paper limit orders the mark crosses.
    paper_engine: Arc<PaperTradingEngine>,
    // Completed 1-minute bars are sent here for continuous ClickHouse persistence.
    bar_sink: BarSink,
    // Live frame bus: every tick becomes a `market.trades` frame so the tape and
    // the watchlist update between minute boundaries.
    live: api::live_bus::LiveSender,
) -> PipelineHandle {
    // Shared-data contract: this pipeline is the mark source for BOTH halves
    // (paper fills + live decisioning).  Registering the instrument's asset
    // class lets the paper half route orders to the right internal account.
    paper_engine.register_instrument(&instrument_id, asset_class);
    // Stage 3 reads the paper account for equity-relative sizing; stage 2 owns
    // the mark board.  One engine, two readers.
    let paper_engine_for_sizing = Arc::clone(&paper_engine);
    let (raw_prod, raw_cons) = rtrb::RingBuffer::<RawTick>::new(4096);
    let (world_prod, world_cons) = rtrb::RingBuffer::<WorldEvent>::new(1024);
    let (intent_prod, intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(256);

    let t1 = tokio::spawn(stage_socket_reader(symbol, asset_class, raw_prod, tee_tx));
    let t2 = tokio::spawn(stage_bar_builder(
        instrument_id.clone(),
        raw_cons,
        world_prod,
        paper_engine,
        bar_sink,
        live,
    ));
    let t3 = tokio::spawn(stage_strategy_eval(
        instrument_id.clone(),
        asset_class,
        world_cons,
        intent_prod,
        Arc::clone(&paper_engine_for_sizing),
    ));
    let t4 = tokio::spawn(stage_risk_exec(
        instrument_id,
        intent_cons,
        execution_engine,
        risk_gate,
    ));

    PipelineHandle {
        _tasks: vec![t1, t2, t3, t4],
    }
}

/// Stage 1: own the WS connection, push `RawTick` into `ring_raw`, tee to JetStream.
/// Dispatches to the appropriate collector based on asset class:
///   - CryptoSpotCex → KrakenCollector (Kraken WS v2)
///   - Equity / Etf   → AlpacaDataCollector (Alpaca WS IEX feed)
async fn stage_socket_reader(
    symbol: String,
    asset_class: AssetClass,
    raw_prod: rtrb::Producer<RawTick>,
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
) {
    info!(
        symbol,
        ?asset_class,
        "hot-path stage 1 (socket-reader) starting"
    );
    let result: Result<(), CollectorError> = match asset_class {
        AssetClass::Equity | AssetClass::Etf => {
            let collector =
                collectors::equity::alpaca_data::AlpacaDataCollector::new(symbol.clone());
            collector.run_in_process(raw_prod, tee_tx).await
        }
        _ => {
            let collector = collectors::crypto::kraken::KrakenCollector::new(symbol.clone());
            collector.run_in_process(raw_prod, tee_tx).await
        }
    };
    if let Err(e) = result {
        warn!(symbol, error = %e, "stage 1 (socket-reader) exited with error");
    }
}

/// Rolling 1-minute OHLCV bucket built from the live trade stream.
///
/// `bucket_start_secs` is the Unix second at the start of the open minute, or
/// `-1` when no bucket is open.  Open/high/low/close track the trade prices and
/// `volume` sums trade sizes; a bucket is finalized into a [`CollectedBar`] when
/// a trade for a later minute arrives or the wall clock passes the minute end.
pub(crate) struct Minute1Aggregator {
    bucket_start_secs: i64,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,
    trade_count: u64,
}

impl Minute1Aggregator {
    pub(crate) fn new() -> Self {
        Self {
            bucket_start_secs: -1,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: Decimal::ZERO,
            trade_count: 0,
        }
    }

    fn start(&mut self, minute: i64, price: Decimal, size: Decimal) {
        self.bucket_start_secs = minute;
        self.open = price;
        self.high = price;
        self.low = price;
        self.close = price;
        self.volume = size;
        self.trade_count = 1;
    }

    /// Build a `CollectedBar` from the current bucket.  `available_time` is the
    /// minute's close (open + 60s), matching the historical collector's
    /// convention; `sequence` is the bar-open epoch second.
    fn finalize(&self) -> CollectedBar {
        CollectedBar {
            available_time: chrono::DateTime::from_timestamp(self.bucket_start_secs + 60, 0)
                .unwrap_or_else(chrono::Utc::now),
            sequence: self.bucket_start_secs.max(0) as u64,
            open: self.open.to_string(),
            high: self.high.to_string(),
            low: self.low.to_string(),
            close: self.close.to_string(),
            volume: self.volume.to_string(),
            trade_count: self.trade_count,
        }
    }

    /// Fold a trade into the aggregator, returning the prior minute's completed
    /// bar when this trade rolls into a new minute.
    pub(crate) fn on_trade(
        &mut self,
        price: Decimal,
        size: Decimal,
        ts_secs: i64,
    ) -> Option<CollectedBar> {
        let minute = ts_secs - ts_secs.rem_euclid(60);
        if self.bucket_start_secs < 0 {
            self.start(minute, price, size);
            return None;
        }
        if minute > self.bucket_start_secs {
            let completed = self.finalize();
            self.start(minute, price, size);
            return Some(completed);
        }
        // Same minute (or a slightly out-of-order earlier tick): extend the bar.
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.volume += size;
        self.trade_count += 1;
        None
    }

    /// Finalize and close the open bucket if the wall clock has passed its minute
    /// end — so bars are persisted promptly even during a lull in trades.
    fn flush_if_elapsed(&mut self, now_secs: i64) -> Option<CollectedBar> {
        if self.bucket_start_secs >= 0 && now_secs >= self.bucket_start_secs + 60 {
            let completed = self.finalize();
            self.bucket_start_secs = -1;
            return Some(completed);
        }
        None
    }
}

/// Render one tape frame as JSON text. The aggressor side is reported as
/// `unknown` rather than guessed — the tape colours by side and a wrong colour
/// is a wrong statement about who lifted the offer.
fn tape_frame_json(
    ts_ms: i64,
    price: domain::money::Price,
    size: domain::money::Size,
    side: domain::payloads::trade::TradeSide,
) -> String {
    use std::fmt::Write;
    let side = match side {
        domain::payloads::trade::TradeSide::Buy => "buy",
        domain::payloads::trade::TradeSide::Sell => "sell",
        _ => "unknown",
    };
    let mut out = String::with_capacity(96);
    // Decimals render as digits, '-' and '.', so no escaping is needed.
    let _ = write!(
        out,
        r#"{{"ts":{ts_ms},"price":"{}","size":"{}","side":"{side}"}}"#,
        price.inner(),
        size.inner()
    );
    out
}

/// What an idle consumer does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdleStep {
    /// Yield to the scheduler and poll again at once.
    Spin,
    /// Park for this long.
    Sleep(std::time::Duration),
}

/// Idle policy for a ring consumer: a **wall-clock-bounded** spin, then park
/// with backoff.
///
/// The three consumers used to `yield_now` in a tight loop, which pins a core
/// at 100% while the market is quiet. A tick arriving within `SPIN_BUDGET` of
/// the last one still sees a spin-fast pop; after that the consumer sleeps,
/// escalating from 200 us to a 1 ms cap, so an idle pipeline costs close to
/// nothing.
///
/// The spin is bounded by time, not by a count of polls: a count of 64 `yield_now`
/// calls is tens of microseconds, which a normal trade stream (a tick every few
/// milliseconds) always outlasts, so every tick would have met a sleeping
/// consumer.
///
/// **A parked wake-up is not 1 ms on every OS.** The sleep is a timer, and the
/// timer's resolution is the OS's: measured on the Windows dev box, a requested
/// 1 ms sleep takes 15.6 ms (`SET-C IDLE` in the metrics test), because the
/// default timer tick is 15.6 ms. On Linux, where the platform runs, it is
/// about the request plus ~60 us. So after a lull a tick can wait up to one
/// timer tick before it is consumed. That is acceptable for the 1-minute bars
/// and the tape, and it is the trade this makes against a pinned core; raising
/// the Windows timer resolution (`timeBeginPeriod(1)`) is a one-line boot call
/// that belongs in `main.rs`.
pub(crate) struct IdleBackoff {
    idle_since: Option<std::time::Instant>,
    sleeps: u32,
}

impl IdleBackoff {
    /// How long an empty ring is polled hot before the consumer parks.
    const SPIN_BUDGET: std::time::Duration = std::time::Duration::from_micros(200);
    const BASE_SLEEP_US: u64 = 100;
    const MAX_SLEEP_US: u64 = 1_000;

    pub(crate) const fn new() -> Self {
        Self {
            idle_since: None,
            sleeps: 0,
        }
    }

    /// A pop succeeded: the next idle period starts spinning again.
    #[inline]
    pub(crate) fn reset(&mut self) {
        self.idle_since = None;
        self.sleeps = 0;
    }

    /// The policy, as a pure function of the clock.
    pub(crate) fn next_step(&mut self, now: std::time::Instant) -> IdleStep {
        let since = *self.idle_since.get_or_insert(now);
        if now.saturating_duration_since(since) < Self::SPIN_BUDGET {
            return IdleStep::Spin;
        }
        self.sleeps = (self.sleeps + 1).min(4);
        let us = (Self::BASE_SLEEP_US << self.sleeps).min(Self::MAX_SLEEP_US);
        IdleStep::Sleep(std::time::Duration::from_micros(us))
    }

    /// The ring was empty: spin, then park with backoff.
    pub(crate) async fn wait(&mut self) {
        match self.next_step(std::time::Instant::now()) {
            IdleStep::Spin => tokio::task::yield_now().await,
            IdleStep::Sleep(d) => tokio::time::sleep(d).await,
        }
    }
}

/// Stage 2: consume `RawTick`, feed the mark board, emit a per-tick `WorldEvent`
/// for live strategy evaluation, AND aggregate trades into 1-minute OHLCV bars
/// that are persisted to ClickHouse continuously (independent of any strategy).
async fn stage_bar_builder(
    instrument_id: String,
    mut raw_cons: rtrb::Consumer<RawTick>,
    mut world_prod: rtrb::Producer<WorldEvent>,
    paper_engine: Arc<PaperTradingEngine>,
    bar_sink: BarSink,
    live: api::live_bus::LiveSender,
) {
    use domain::money::Size;
    use domain::payloads::bar::{BarPayload, Timeframe};
    use domain::payloads::trade::TradePayload;

    use chrono::DateTime;

    info!(instrument_id, "hot-path stage 2 (bar-builder) starting");

    let mut agg = Minute1Aggregator::new();
    let emit_bar = |bar: CollectedBar| {
        // v3 deployments decide on completed minutes (Set U U-4.1).
        crate::live_bars::publish(&instrument_id, &bar);
        let _ = bar_sink.tx.send(PersistBar {
            instrument_id: instrument_id.clone(),
            venue_id: bar_sink.venue_id.clone(),
            source: bar_sink.source.clone(),
            trust_tier: bar_sink.trust_tier.clone(),
            bar,
        });
    };

    let mut idle = IdleBackoff::new();
    loop {
        match raw_cons.pop() {
            Ok(tick) => {
                idle.reset();
                let trade = match tick.decode_payload::<TradePayload>() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let price = trade.price;
                paper_engine.on_mark(&instrument_id, price);

                // The tape. Published per tick so time & sales is genuinely live
                // rather than a minute behind the market. No `serde_json::Value`
                // is built on this path: the frame is written straight into one
                // `String`, and only when a socket is actually listening.
                if api::live_bus::has_listeners(&live) {
                    api::live_bus::publish_text(
                        &live,
                        domain::lanes::MARKET_TRADES,
                        &instrument_id,
                        tape_frame_json(
                            tick.timestamp_ns / 1_000_000,
                            price,
                            trade.size,
                            trade.side,
                        ),
                    );
                }

                // Aggregate into the 1-minute bar; persist a completed minute.
                let ts_secs = tick.timestamp_ns / 1_000_000_000;
                if let Some(completed) = agg.on_trade(price.inner(), trade.size.inner(), ts_secs) {
                    emit_bar(completed);
                }

                // Per-tick snapshot keeps the live strategy pipeline responsive.
                let bar = BarPayload::new(
                    Timeframe::Minutes1,
                    price,
                    price,
                    price,
                    price,
                    Size::from_decimal(Decimal::ONE),
                    tick.sequence,
                );
                let nanos = (tick.timestamp_ns % 1_000_000_000).unsigned_abs() as u32;
                let available_time =
                    DateTime::from_timestamp(ts_secs, nanos).unwrap_or_else(chrono::Utc::now);
                let event = WorldEvent::Bar {
                    instrument_id: instrument_id.clone(),
                    timeframe: Timeframe::Minutes1,
                    bar,
                    available_time,
                };
                if world_prod.push(event).is_err() {
                    warn!(instrument_id, "ring_world full — WorldEvent dropped");
                }
            }
            Err(_) => {
                // No tick pending: flush a completed minute even without new
                // trades, then yield.
                if let Some(completed) = agg.flush_if_elapsed(chrono::Utc::now().timestamp()) {
                    emit_bar(completed);
                }
                idle.wait().await;
            }
        }
    }
}

/// Stage 3: consume `WorldEvent`, run strategy evaluation, push `OrderIntent`.
///
/// This stage held `let mut strategy: Option<StrategyInstance> = None;` — a
/// started strategy returned `200` and then produced no intents, for ever,
/// with nothing in the logs to say so.  It now owns the instances for this
/// instrument and re-syncs them from [`strategy_runtime::live`] whenever the
/// registry's generation moves.
///
/// Per `WorldEvent` this costs one relaxed atomic load and no allocation:
/// the instances are owned here (they hold mutable per-instrument state, so
/// they cannot be shared), and the registry is read only on a generation
/// change, which is a cold path (Set E locked decision 12).
async fn stage_strategy_eval(
    instrument_id: String,
    asset_class: AssetClass,
    mut world_cons: rtrb::Consumer<WorldEvent>,
    mut intent_prod: rtrb::Producer<OrderIntent>,
    paper_engine: Arc<PaperTradingEngine>,
) {
    use strategy_runtime::live::{LiveKey, LiveStrategySpec};
    use strategy_runtime::runtime::StrategyInstance;
    use strategy_runtime::SizingContext;

    info!(instrument_id, "hot-path stage 3 (strategy-eval) starting");

    let registry = strategy_runtime::live::global();
    // (key, instance, needs_sizing) — owned here, rebuilt only on a change.
    let mut instances: Vec<(LiveKey, StrategyInstance)> = Vec::new();
    // `u64::MAX` forces a first sync even when the registry is still at 0, so
    // a strategy armed before this pipeline started is picked up immediately.
    let mut synced_generation: u64 = u64::MAX;

    let mut idle = IdleBackoff::new();
    loop {
        // One `Acquire` load per iteration.  No lock, no allocation.
        let generation = registry.generation();
        if generation != synced_generation {
            let declared: Vec<Arc<LiveStrategySpec>> = registry.snapshot_for(&instrument_id);
            resync_instances(&instrument_id, &mut instances, &declared);
            synced_generation = generation;
        }

        match world_cons.pop() {
            Ok(event) => {
                idle.reset();
                if instances.is_empty() {
                    continue;
                }
                for (key, instance) in instances.iter_mut() {
                    // Equity-relative sizing needs account state.  Read it only
                    // for definitions that declare such a size mode, so the
                    // common fixed-size strategy costs no account lock here.
                    if instance.needs_sizing_context() {
                        // The lot comes from instrument metadata, not from a
                        // guess: quantizing to an assumed lot is how a size the
                        // venue will reject gets submitted.  No metadata means
                        // no sizing context, which the sizer reports as a
                        // refused order naming what was missing.
                        let sizing = match (
                            paper_engine.mark(&instrument_id),
                            risk::context::instrument_limits(&instrument_id),
                        ) {
                            (Some(mark), risk::Measured::Yes(limits)) => Some(SizingContext::new(
                                paper_engine.snapshot(asset_class).equity,
                                mark,
                                limits.lot_size,
                            )),
                            _ => None,
                        };
                        instance.set_sizing_context(sizing);
                    }

                    let outcome = instance.process_event(&event);

                    // A size that could not be computed is a refusal, not a
                    // non-event.  It is logged with the strategy that asked.
                    for (signal, error) in &outcome.errors {
                        warn!(
                            instrument_id,
                            strategy_id = %key.strategy_id,
                            signal,
                            error = %error,
                            "strategy signal produced no order — sizing failed"
                        );
                    }

                    for intent in outcome.intents {
                        if intent_prod.push(intent).is_err() {
                            warn!(instrument_id, "ring_intent full — intent dropped");
                        }
                    }
                }
            }
            Err(_) => idle.wait().await,
        }
    }
}

/// Bring the owned instance set in line with what the registry declares.
///
/// Instances whose key is still declared are **kept**, warm state and all:
/// starting a second strategy on an instrument must not reset the first one's
/// indicator history.
fn resync_instances(
    instrument_id: &str,
    instances: &mut Vec<(
        strategy_runtime::live::LiveKey,
        strategy_runtime::runtime::StrategyInstance,
    )>,
    declared: &[Arc<strategy_runtime::live::LiveStrategySpec>],
) {
    use strategy_runtime::runtime::StrategyInstance;

    let wanted: Vec<strategy_runtime::live::LiveKey> = declared.iter().map(|s| s.key()).collect();

    let before = instances.len();
    instances.retain(|(key, _)| wanted.contains(key));
    let removed = before - instances.len();

    let mut added = 0;
    for spec in declared {
        let key = spec.key();
        if instances.iter().any(|(k, _)| *k == key) {
            continue;
        }
        instances.push((
            key,
            StrategyInstance::new(
                spec.user_id.clone(),
                spec.instrument_id.clone(),
                spec.definition.clone(),
                chrono::Utc::now(),
            ),
        ));
        added += 1;
    }

    if added > 0 || removed > 0 {
        info!(
            instrument_id,
            added,
            removed,
            live = instances.len(),
            "hot-path stage 3 re-synced its strategy set"
        );
    }
}

/// Stage 4: consume `OrderIntent`, apply risk gate, submit to execution engine.
///
/// The context used to be eight hardcoded constants — a zero position, no
/// mark, a tick and lot of `1`, a zero daily loss, `0, 0` order counts — with
/// a comment blaming latency.  Three limits could not fire against it, so
/// wiring stage 3 on top would have turned the single chokepoint into a rubber
/// stamp (FM-2).  It now asks the one process-wide provider, and a context
/// that cannot be built is a refusal, not a permissive default.
async fn stage_risk_exec(
    instrument_id: String,
    mut intent_cons: rtrb::Consumer<OrderIntent>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
) {
    info!(instrument_id, "hot-path stage 4 (risk/exec) starting");
    let mut idle = IdleBackoff::new();
    loop {
        match intent_cons.pop() {
            Ok(intent) => {
                idle.reset();
                let ctx = match risk::context::context_for(&intent) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        // Failing closed: no order beats an order through an
                        // unverified gate, and the reason is visible.
                        warn!(
                            instrument_id,
                            strategy_id = ?intent.strategy_id,
                            error = %e,
                            "no risk context — intent refused before the gate"
                        );
                        continue;
                    }
                };
                match risk_gate.check(intent, &ctx) {
                    Ok(approved) => {
                        if let Err(e) = execution_engine.submit(approved).await {
                            warn!(instrument_id, error = %e, "order submission failed");
                        }
                    }
                    Err(e) => {
                        warn!(instrument_id, error = %e, "risk gate rejected intent");
                    }
                }
            }
            Err(_) => idle.wait().await,
        }
    }
}

// ── The execution path to a fill ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;
    use std::sync::Mutex;
    use std::time::Duration;

    use api::instrument_meta::InstrumentMeta;
    use domain::instrument::{HaltPolicy, TradingSchedule};
    use domain::money::{Price, Size};
    use domain::order::{OrderIntent, OrderState, OrderType, Side};
    use domain::payloads::bar::{BarPayload, Timeframe};
    use domain::strategy_def::StrategyDefinition;
    use domain::{RiskRejection, TrustTier};
    use execution::journal;
    use risk::{GlobalRiskLimits, KillSwitch, RateKey, RiskGate};
    use strategy_runtime::live::{LiveSource, LiveStrategySpec};
    use strategy_runtime::world::WorldEvent;

    // The process-wide pieces live in `crate::test_support`, shared with the
    // live-runner tests: `risk::context::install` and `journal::install` are
    // first-install-wins, so one binary can hold only one of each.
    use crate::test_support::{env, ready_instrument};

    fn gate_with(limits: GlobalRiskLimits) -> Arc<RiskGate> {
        Arc::new(RiskGate::new(limits, Arc::new(KillSwitch::new(false))))
    }

    fn engine() -> Arc<execution::ExecutionEngine> {
        Arc::new(execution::ExecutionEngine::new(Arc::new(
            env().paper.multi_asset_broker(),
        )))
    }

    /// A hand-authored EMA-cross strategy — the kind a person builds in the UI
    /// and the kind ROADMAP §6 assumes can be deployed to paper "in a day".
    fn ema_cross(strategy_id: &str, size_mode: &str, size: &str) -> StrategyDefinition {
        serde_json::from_value(serde_json::json!({
            "strategy_id": strategy_id,
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "min_trust_tier": "centralized_exchange",
            "inputs": [
                { "lane": "features.technical", "instrument": "$bound_at_init",
                  "features": ["ema_7", "ema_21"] }
            ],
            "nodes": [
                { "id": "c1", "type": "condition",
                  "expr": "feature('ema_7') > feature('ema_21')" },
                { "id": "s1", "type": "signal", "when": "c1", "emit": "long" }
            ],
            "actions": [
                { "on_signal": "long", "type": "place_order",
                  "order": { "side": "buy", "size_mode": size_mode, "size": size } }
            ]
        }))
        .expect("hand-authored definition must parse")
    }

    fn feature(instrument_id: &str, name: &str, value: f64) -> WorldEvent {
        WorldEvent::Feature {
            instrument_id: instrument_id.to_owned(),
            feature_value: strategy_runtime::FeatureValue::new(name, value, 1, chrono::Utc::now()),
        }
    }

    fn bar(instrument_id: &str, price: &str) -> WorldEvent {
        let p = Price::from_str(price).unwrap();
        WorldEvent::Bar {
            instrument_id: instrument_id.to_owned(),
            timeframe: Timeframe::Minutes1,
            bar: BarPayload::new(
                Timeframe::Minutes1,
                p,
                p,
                p,
                p,
                Size::from_str("1").unwrap(),
                1,
            ),
            available_time: chrono::Utc::now(),
        }
    }

    /// Wait for `f` to return `Some`, or fail.  The stages are infinite loops,
    /// so the test observes their effect rather than awaiting their completion.
    async fn eventually<T, F: FnMut() -> Option<T>>(what: &str, mut f: F) -> T {
        for _ in 0..400 {
            if let Some(v) = f() {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn journal_order(key: journal::Uuid) -> Option<journal::RecordedOrder> {
        journal::require().unwrap().find_order(key).await.unwrap()
    }

    async fn journal_fills(key: journal::Uuid) -> Vec<journal::PersistedFill> {
        journal::require().unwrap().fills_for(key).await.unwrap()
    }

    // ── The definition of done ───────────────────────────────────────────────

    /// A hand-authored strategy, declared live through the same registry
    /// `POST /api/strategies/:id/start` writes to, produces an intent from the
    /// real stage 3, clears the real risk gate against a **measured** context,
    /// becomes a paper order, fills, and both the order and the fill are in the
    /// journal.
    ///
    /// Every one of those steps was missing or fabricated before track 16:
    /// stage 3 was `Option::None`, the gate's context was eight constants, and
    /// no code anywhere wrote an order or a fill.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_hand_authored_strategy_reaches_a_persisted_fill() {
        let instrument = "E2E-BTC-USD";
        ready_instrument(instrument, "50000");

        strategy_runtime::live::global()
            .register(LiveStrategySpec {
                user_id: "operator".into(),
                instrument_id: instrument.into(),
                definition: ema_cross("e2e_ema_cross", "fixed", "0.01"),
                source: LiveSource::Strategy {
                    strategy_id: "e2e_ema_cross".into(),
                },
            })
            .expect("declare the strategy live");

        let (mut world_prod, world_cons) = rtrb::RingBuffer::<WorldEvent>::new(64);
        let (intent_prod, intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(64);

        let _t3 = tokio::spawn(stage_strategy_eval(
            instrument.to_string(),
            AssetClass::CryptoSpotCex,
            world_cons,
            intent_prod,
            Arc::clone(&env().paper),
        ));
        let _t4 = tokio::spawn(stage_risk_exec(
            instrument.to_string(),
            intent_cons,
            engine(),
            gate_with(GlobalRiskLimits::default()),
        ));

        // ema_7 > ema_21 — the cross fires.
        world_prod.push(feature(instrument, "ema_7", 11.0)).unwrap();
        world_prod
            .push(feature(instrument, "ema_21", 10.0))
            .unwrap();
        world_prod.push(bar(instrument, "50000")).unwrap();

        // The observable result: a filled paper order on this instrument.
        let order = eventually("a paper order", || {
            env()
                .paper
                .orders_for_instrument(instrument)
                .into_iter()
                .next()
        })
        .await;

        assert_eq!(order.side, "buy");
        assert_eq!(
            order.filled_qty,
            Decimal::from_str("0.01").unwrap(),
            "a market order on a marked instrument must fill in full"
        );

        // And the durable half: the order and the fill are both recorded.
        let key = order.idempotency_key;
        let recorded = eventually("the order in the journal", || {
            futures_block(journal_order(key))
        })
        .await;
        assert_eq!(recorded.state, OrderState::Filled);
        assert!(
            recorded.broker_order_id.is_some(),
            "a submitted order must carry its broker id"
        );

        let fills = journal_fills(key).await;
        assert_eq!(fills.len(), 1, "exactly one fill recorded");
        assert_eq!(fills[0].instrument_id, instrument);
        assert_eq!(fills[0].side, Side::Buy);
        assert!(fills[0].fill_price.inner() > Decimal::ZERO);

        // The fee the paper venue charged is recorded, not zeroed.  A cost that
        // reads as zero when it was merely unknown makes a forward fill stream
        // look cheaper than the backtest that justified deploying it, which is
        // the one comparison Gate 15 exists to make.
        let commission = fills[0]
            .commission
            .expect("the paper venue charges a fee and therefore knows it");
        assert!(
            commission > Decimal::ZERO,
            "a crypto-spot taker fill carries a taker fee, got {commission}"
        );
    }

    /// Helper: run a future to completion on the current thread from inside a
    /// synchronous closure.  Only used by `eventually`, which polls.
    fn futures_block<T, F: std::future::Future<Output = Option<T>>>(f: F) -> Option<T> {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }

    // ── SC-7: idempotent across a restart ────────────────────────────────────

    /// Resubmitting an order after every in-memory cache is gone must not
    /// double-fill.  This is the restart case SC-7 is about: the old
    /// implementation's only defence was a `VecDeque` inside `ExecutionEngine`,
    /// which a restart wipes.  Here both the engine and the gate are rebuilt —
    /// only the journal survives, which is exactly what survives a restart.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resubmit_after_a_restart_does_not_double_fill() {
        let instrument = "IDEM-BTC-USD";
        ready_instrument(instrument, "50000");

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let key = intent.idempotency_key;

        // First submission, through a real gate and a real context.
        let gate = gate_with(GlobalRiskLimits::default());
        let ctx = risk::context::context_for(&intent).expect("a measured context");
        let approved = gate.check(intent.clone(), &ctx).expect("gate approves");
        let first = engine().submit(approved).await.expect("submitted");
        assert!(!first.duplicate);
        assert!(first.fill.is_some(), "a paper market order fills on submit");

        // "Restart": new engine (fresh FillProcessor), new gate (fresh decision
        // cache).  The journal is the only thing that persists.
        let gate2 = gate_with(GlobalRiskLimits::default());
        let ctx2 = risk::context::context_for(&intent).expect("a measured context");
        let approved2 = gate2.check(intent, &ctx2).expect("gate approves");
        let second = engine().submit(approved2).await.expect("resubmitted");

        assert!(
            second.duplicate,
            "the journal must recognise the key across a restart"
        );
        assert_eq!(
            second.broker_order_id, first.broker_order_id,
            "a redelivery returns the original broker order id"
        );
        assert_eq!(
            journal_fills(key).await.len(),
            1,
            "a redelivery must not record a second fill"
        );
    }

    // ── Part 2: the limits actually fire ─────────────────────────────────────

    /// The per-second order-rate limit fires against counts the **gate** keeps.
    /// Both order doors used to pass `0, 0`, so this limit could not fire.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_rate_limit_fires_when_breached() {
        let instrument = "RATE-BTC-USD";
        ready_instrument(instrument, "50000");

        let limit = 3u32;
        let gate = gate_with(GlobalRiskLimits {
            max_orders_per_second: limit,
            ..Default::default()
        });

        let mut approvals = 0;
        let mut rejection = None;
        for _ in 0..(limit + 2) {
            let intent = OrderIntent::new(
                instrument,
                Side::Buy,
                OrderType::Market,
                Size::from_str("0.001").unwrap(),
                None,
                None,
            );
            let ctx = risk::context::context_for(&intent).expect("a measured context");
            match gate.check(intent, &ctx) {
                Ok(_) => approvals += 1,
                Err(e) => {
                    rejection = Some(e);
                    break;
                }
            }
        }

        assert_eq!(approvals, limit as usize, "the gate stops at the burst");
        assert!(
            matches!(
                rejection,
                Some(RiskRejection::RateLimitPerSecondExceeded { .. })
            ),
            "expected the per-second limit to fire, got {rejection:?}"
        );
        // And the count is the gate's own, not the caller's.
        assert_eq!(
            gate.rate_counts(&RateKey::manual(instrument))
                .unwrap()
                .last_second,
            limit
        );
    }

    /// The daily-loss limit fires on a **realized** loss the paper account
    /// actually took.  Both doors used to pass `Decimal::ZERO`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_daily_loss_limit_fires_on_a_real_loss() {
        let instrument = "LOSS-BTC-USD";
        ready_instrument(instrument, "50000");
        let e = env();

        // Buy, then sell after the mark falls: a realized loss on the account.
        let open = risk::ApprovedOrder::new_for_test(OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.05").unwrap(),
            None,
            None,
        ));
        engine().submit(open).await.expect("opening fill");

        e.paper
            .on_mark(instrument, Price::from_str("40000").unwrap());

        let close = risk::ApprovedOrder::new_for_test(OrderIntent::new(
            instrument,
            Side::Sell,
            OrderType::Market,
            Size::from_str("0.05").unwrap(),
            None,
            None,
        ));
        engine().submit(close).await.expect("closing fill");

        let loss = e.paper.daily_loss_usd(AssetClass::CryptoSpotCex);
        assert!(
            loss > Decimal::ZERO,
            "the round trip must have realized a loss, got {loss}"
        );

        // A limit below the loss the account took must refuse the next order.
        let gate = gate_with(GlobalRiskLimits {
            max_daily_loss_usd: loss - Decimal::ONE,
            ..Default::default()
        });
        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.001").unwrap(),
            None,
            None,
        );
        let ctx = risk::context::context_for(&intent).expect("a measured context");
        let err = gate.check(intent, &ctx).unwrap_err();
        assert!(
            matches!(err, RiskRejection::DailyLossLimitExceeded { .. }),
            "expected the daily-loss limit to fire, got {err}"
        );
    }

    /// An instrument whose venue increments are unknown is refused, not judged
    /// against a zero tick and lot (which disable both checks).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_instrument_without_venue_increments_is_refused() {
        let instrument = "NOMETA-BTC-USD";
        // Registered and marked, but deliberately no `instruments` metadata.
        env()
            .paper
            .register_instrument(instrument, AssetClass::CryptoSpotCex);
        env()
            .paper
            .on_mark(instrument, Price::from_str("50000").unwrap());

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let ctx = risk::context::context_for(&intent).expect("context builds");
        let err = gate_with(GlobalRiskLimits::default())
            .check(intent, &ctx)
            .unwrap_err();
        assert!(
            matches!(err, RiskRejection::InstrumentMetadataUnavailable { .. }),
            "expected a refusal naming the missing metadata, got {err}"
        );
    }

    /// An instrument with no mark is refused rather than having its exposure
    /// and price-sanity checks quietly skipped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_instrument_without_a_mark_is_refused() {
        let instrument = "NOMARK-BTC-USD";
        env()
            .paper
            .register_instrument(instrument, AssetClass::CryptoSpotCex);
        env().meta.put(InstrumentMeta {
            venue_id: "paper".into(),
            instrument_id: instrument.to_owned(),
            tick_size: Decimal::from_str("0.1").unwrap(),
            lot_size: Decimal::from_str("0.00000001").unwrap(),
            trading_hours: TradingSchedule::always_open(),
            halt_policy: HaltPolicy::NonHaltable,
            trust_tier: TrustTier::CentralizedExchange,
            active: true,
        });

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let ctx = risk::context::context_for(&intent).expect("context builds");
        let err = gate_with(GlobalRiskLimits::default())
            .check(intent, &ctx)
            .unwrap_err();
        assert!(
            matches!(err, RiskRejection::MarketDataUnavailable { .. }),
            "expected a refusal naming the missing mark, got {err}"
        );
    }

    /// A rejected order reaches no venue and leaves no record: stage 4 must not
    /// submit what the gate refused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gate_rejection_never_reaches_the_venue() {
        let instrument = "REJECT-BTC-USD";
        ready_instrument(instrument, "50000");

        let ks = Arc::new(KillSwitch::new(false));
        let gate = Arc::new(RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks)));
        ks.trip();

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let key = intent.idempotency_key;

        let (mut intent_prod, intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(8);
        let _t4 = tokio::spawn(stage_risk_exec(
            instrument.to_string(),
            intent_cons,
            engine(),
            gate,
        ));
        intent_prod.push(intent).unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            journal_order(key).await.is_none(),
            "a gate rejection must leave no order record"
        );
        assert!(
            env().paper.orders_for_instrument(instrument).is_empty(),
            "a gate rejection must never reach the venue"
        );
    }

    /// Both order doors get their context from the same provider, so a limit
    /// that binds a manual order binds a strategy order identically (SC-2,
    /// COMP-002 §3.1 "both paths converge on the same check").
    ///
    /// The one thing that differs is deliberate: the order-rate **scope**.  A
    /// strategy counts against its own budget, so it cannot get a second
    /// allowance by also placing orders by hand, and a human cannot be locked
    /// out by a chatty strategy.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn both_order_doors_share_one_risk_context() {
        let instrument = "DOORS-BTC-USD";
        ready_instrument(instrument, "50000");

        let manual = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None, // no strategy — the REST door
        );
        let automated = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            Some("some_strategy".to_owned()), // the hot path
        );

        let a = risk::context::context_for(&manual).expect("manual context");
        let b = risk::context::context_for(&automated).expect("strategy context");

        assert_eq!(a.current_position, b.current_position);
        assert_eq!(a.instrument_active, b.instrument_active);
        assert_eq!(a.is_in_session, b.is_in_session);
        assert!(a.market_price.is_available() && b.market_price.is_available());
        assert_eq!(a.instrument_limits, b.instrument_limits);
        // Both doors read today's loss from the same account.  The *value* is
        // not compared: it is per asset class, so a concurrent test's fill can
        // move it between these two calls — what matters is that neither door
        // is handed a default.
        assert!(
            a.daily_loss_usd.is_available() && b.daily_loss_usd.is_available(),
            "neither door may be given an unmeasured daily loss"
        );

        // Different rate scopes, by design.
        let (risk::RateSource::Tracked(ka), risk::RateSource::Tracked(kb)) = (&a.rate, &b.rate)
        else {
            panic!("both doors must use gate-tracked rate counts, never supplied ones");
        };
        assert_ne!(ka.as_str(), kb.as_str());
        assert!(ka.as_str().starts_with("manual:"));
        assert!(kb.as_str().starts_with("strategy:some_strategy:"));
    }

    // ── Part 1: percent-of-balance sizes, or says why it cannot ──────────────

    /// `percent_of_balance` used to return `None` — no order and no error.
    /// It now sizes against the account's equity and the instrument's mark.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn percent_of_balance_sizes_a_real_order() {
        let instrument = "POB-BTC-USD";
        ready_instrument(instrument, "50000");

        strategy_runtime::live::global()
            .register(LiveStrategySpec {
                user_id: "operator".into(),
                instrument_id: instrument.into(),
                definition: ema_cross("pob_ema_cross", "percent_of_balance", "0.001"),
                source: LiveSource::Strategy {
                    strategy_id: "pob_ema_cross".into(),
                },
            })
            .expect("declare the strategy live");

        let (mut world_prod, world_cons) = rtrb::RingBuffer::<WorldEvent>::new(64);
        let (intent_prod, mut intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(64);

        let _t3 = tokio::spawn(stage_strategy_eval(
            instrument.to_string(),
            AssetClass::CryptoSpotCex,
            world_cons,
            intent_prod,
            Arc::clone(&env().paper),
        ));

        world_prod.push(feature(instrument, "ema_7", 11.0)).unwrap();
        world_prod
            .push(feature(instrument, "ema_21", 10.0))
            .unwrap();

        let intent = eventually("a percent-of-balance intent", || intent_cons.pop().ok()).await;
        assert!(
            intent.size.inner() > Decimal::ZERO,
            "percent-of-balance must produce a positive size, not silence"
        );
        // The size is a fraction of equity at the instrument's mark, quantized
        // down to the venue's lot.  The exact arithmetic is pinned by
        // `strategy_runtime::intents::tests`; asserted here is the behaviour
        // that was missing — a real quantity derived from real account state.
        //
        // The account is shared with the other tests in this binary, so the
        // notional is checked as a band around the declared 0.1%, not against
        // an equity figure read at a different instant.
        let lot = Decimal::from_str("0.00000001").unwrap();
        assert_eq!(
            intent.size.inner() % lot,
            Decimal::ZERO,
            "the size must land on the venue's lot"
        );
        let notional = intent.size.inner() * Decimal::from_str("50000").unwrap();
        let equity = env().paper.snapshot(AssetClass::CryptoSpotCex).equity;
        let fraction = notional / equity;
        assert!(
            fraction > Decimal::from_str("0.0005").unwrap()
                && fraction < Decimal::from_str("0.002").unwrap(),
            "0.1% of equity expected; sized {} of equity ({notional} of {equity})",
            fraction
        );
    }

    /// A venue rejection is recorded **as a rejection**, and reusing its key is
    /// refused with the reason.
    ///
    /// If a refused order were left at `accepted` it would be indistinguishable
    /// from an order whose ack went missing — and those need opposite responses
    /// (one is safe to retry with a new key, the other must be reconciled
    /// before anything is retried at all; COMP-002 §3.4 and §3.6).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_venue_rejection_is_recorded_and_the_key_is_not_reusable() {
        let instrument = "BROKE-BTC-USD";
        ready_instrument(instrument, "50000");

        // Far beyond the paper account's cash.  The gate is bypassed on purpose:
        // this exercises the execution engine's handling of a venue refusal, not
        // the gate's position limit (which would catch this first in production,
        // and does — see `the_rate_limit_fires_when_breached` for the gate path).
        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("100000").unwrap(),
            None,
            None,
        );
        let key = intent.idempotency_key;
        let err = engine()
            .submit(risk::ApprovedOrder::new_for_test(intent.clone()))
            .await
            .expect_err("the venue must refuse an order it cannot fund");
        assert!(
            err.to_string().contains("broker"),
            "expected a broker refusal, got {err}"
        );

        let recorded = journal_order(key).await.expect("the attempt is recorded");
        assert_eq!(
            recorded.state,
            OrderState::Rejected,
            "a refused order must not be left looking like a missing ack"
        );

        let again = engine()
            .submit(risk::ApprovedOrder::new_for_test(intent))
            .await
            .expect_err("the key is spent");
        assert!(
            again.to_string().contains("new idempotency key"),
            "the refusal must say how to retry, got {again}"
        );
        assert!(
            journal_fills(key).await.is_empty(),
            "a rejected order has no fills"
        );
    }

    // ── The fill stream track 5 consumes ─────────────────────────────────────

    /// A registered [`execution::fills::FillObserver`] receives every genuinely
    /// new fill exactly once, and is **not** fired again by a resubmit of the
    /// same order.  Reconciliation, the forward-observation collector and
    /// Gate 15's evidence path all hang off this seam, and a fill counted twice
    /// is a forward observation that never happened.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_fill_stream_fires_once_per_new_fill() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Default)]
        struct Counter {
            seen: AtomicUsize,
            last_key: Mutex<Option<journal::Uuid>>,
            /// The one key this test cares about; see `on_fill`.
            watching: Mutex<Option<journal::Uuid>>,
        }
        impl execution::fills::FillObserver for Counter {
            fn on_fill(&self, fill: &execution::fills::FillEvent) {
                // Only this test's own order. The observer registry is global
                // and the suite runs in one binary, so an unfiltered counter
                // counts every other test's fills too and the assertion below
                // fails on how busy the suite is rather than on the seam.
                if Some(fill.idempotency_key) != *self.watching.lock().unwrap() {
                    return;
                }
                self.seen.fetch_add(1, Ordering::SeqCst);
                *self.last_key.lock().unwrap() = Some(fill.idempotency_key);
            }
        }

        let instrument = "STREAM-BTC-USD";
        ready_instrument(instrument, "50000");

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let key = intent.idempotency_key;

        let counter = Arc::new(Counter::default());
        *counter.watching.lock().unwrap() = Some(key);
        execution::fills::register_observer(Arc::clone(&counter) as Arc<_>);
        let before = counter.seen.load(Ordering::SeqCst);

        let gate = gate_with(GlobalRiskLimits::default());

        let ctx = risk::context::context_for(&intent).expect("a measured context");
        engine()
            .submit(gate.check(intent.clone(), &ctx).expect("approved"))
            .await
            .expect("submitted");

        let after_first = counter.seen.load(Ordering::SeqCst);
        assert_eq!(after_first, before + 1, "one fill, one notification");
        assert_eq!(*counter.last_key.lock().unwrap(), Some(key));

        // Resubmit the same order with a fresh engine and gate — the restart
        // case.  The journal recognises it, so no second notification.
        let gate2 = gate_with(GlobalRiskLimits::default());
        let ctx2 = risk::context::context_for(&intent).expect("a measured context");
        engine()
            .submit(gate2.check(intent, &ctx2).expect("approved"))
            .await
            .expect("resubmitted");

        assert_eq!(
            counter.seen.load(Ordering::SeqCst),
            after_first,
            "a redelivered order must not publish a second fill"
        );
    }

    // ── Stage 3's re-sync ────────────────────────────────────────────────────

    #[test]
    fn resync_keeps_warm_instances_and_adds_new_ones() {
        let instrument = "RESYNC-BTC-USD";
        let mut instances: Vec<(
            strategy_runtime::live::LiveKey,
            strategy_runtime::runtime::StrategyInstance,
        )> = Vec::new();

        let a = Arc::new(LiveStrategySpec {
            user_id: "u".into(),
            instrument_id: instrument.into(),
            definition: ema_cross("resync_a", "fixed", "0.01"),
            source: LiveSource::Strategy {
                strategy_id: "resync_a".into(),
            },
        });
        let b = Arc::new(LiveStrategySpec {
            user_id: "u".into(),
            instrument_id: instrument.into(),
            definition: ema_cross("resync_b", "fixed", "0.01"),
            source: LiveSource::Strategy {
                strategy_id: "resync_b".into(),
            },
        });

        resync_instances(instrument, &mut instances, &[Arc::clone(&a)]);
        assert_eq!(instances.len(), 1);

        // Warm the first instance up so a re-sync that keeps it is observable.
        //
        // Observable *behaviourally*, not by address: the instances live in a
        // `Vec`, so pushing the second one reallocates and every element moves
        // whether or not it was rebuilt. The earlier form of this test compared
        // `addr_of!(instances[i].1)` across that push and was asserting the
        // allocator's behaviour, not the resync's.
        //
        // What "kept" actually means is that `ema_7` is still set. A rebuilt
        // instance starts with no features, so the cross below cannot fire on
        // one — `ema_21` alone is not a crossing.
        instances[0]
            .1
            .process_event(&feature(instrument, "ema_7", 11.0));

        resync_instances(instrument, &mut instances, &[a, b]);
        assert_eq!(instances.len(), 2, "the second strategy is added");
        let still_warm = instances
            .iter()
            .position(|(k, _)| k.strategy_id == "resync_a")
            .expect("the first strategy is still there");
        let fresh = instances
            .iter()
            .position(|(k, _)| k.strategy_id == "resync_b")
            .expect("the second strategy was added");

        let kept = instances[still_warm]
            .1
            .process_event(&feature(instrument, "ema_21", 10.0));
        assert_eq!(
            kept.intents.len(),
            1,
            "adding a strategy must not rebuild the one already running: the              kept instance still had `ema_7` and crossed"
        );
        let rebuilt = instances[fresh]
            .1
            .process_event(&feature(instrument, "ema_21", 10.0));
        assert!(
            rebuilt.intents.is_empty(),
            "and the new one is genuinely new: one feature is not a crossing"
        );

        resync_instances(instrument, &mut instances, &[]);
        assert!(instances.is_empty(), "stopping removes every instance");
    }

    // ── Paper only ───────────────────────────────────────────────────────────

    /// No live-venue path exists from here.  The engine this process submits
    /// through is the in-house paper engine, and the journal records every
    /// order as `paper` — which migration 0121 also enforces with a CHECK.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execution_is_paper_only() {
        assert_eq!(journal::PAPER_MODE, "paper");

        let instrument = "PAPER-BTC-USD";
        ready_instrument(instrument, "50000");

        let intent = OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        let ctx = risk::context::context_for(&intent).expect("a measured context");
        let approved = gate_with(GlobalRiskLimits::default())
            .check(intent, &ctx)
            .expect("approved");
        let result = engine().submit(approved).await.expect("submitted");

        // The fill came from the in-house engine: the order id it minted is the
        // one the journal holds, and the same order is visible on the paper
        // account.  Nothing left this process.
        assert!(env()
            .paper
            .orders_for_instrument(instrument)
            .iter()
            .any(|o| o.order_id == result.broker_order_id));
    }
}

// ── Set C metrics: allocations and latency per tick ──────────────────────────

/// Test-only allocator that counts allocations **on the calling thread**, so a
/// measurement is not polluted by the other tests running in parallel.
#[cfg(test)]
mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ALLOCS: Cell<u64> = const { Cell::new(0) };
    }

    pub struct Counting;

    // SAFETY: delegates every call to `System`; the counter is a const-init
    // thread-local `Cell`, which never allocates.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            System.alloc(l)
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            System.dealloc(p, l)
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            System.realloc(p, l, n)
        }
    }

    pub fn count() -> u64 {
        ALLOCS.with(Cell::get)
    }
}

#[cfg(test)]
#[global_allocator]
static COUNTING: alloc_count::Counting = alloc_count::Counting;

#[cfg(test)]
mod metrics {
    use super::*;
    use std::str::FromStr;

    use domain::money::{Price, Size};
    use domain::payloads::trade::TradeSide;

    #[test]
    fn tape_frame_is_valid_json_with_the_published_shape() {
        let p = Price::from_str("50000.25").unwrap();
        let z = Size::from_str("0.5").unwrap();
        for (side, want) in [
            (TradeSide::Buy, "buy"),
            (TradeSide::Sell, "sell"),
            (TradeSide::Unknown, "unknown"),
        ] {
            let v: serde_json::Value =
                serde_json::from_str(&tape_frame_json(1_700_000_000_123, p, z, side)).unwrap();
            assert_eq!(v["ts"], 1_700_000_000_123_i64);
            assert_eq!(v["price"], "50000.25");
            assert_eq!(v["size"], "0.5");
            assert_eq!(v["side"], want);
        }
    }

    #[test]
    fn idle_backoff_spins_then_sleeps_then_caps_and_resets() {
        use std::time::Duration;
        let base = std::time::Instant::now();
        let at = |us: u64| base + Duration::from_micros(us);
        let mut b = IdleBackoff::new();
        // Within the spin budget it never sleeps, however many times it is asked.
        for us in [0, 1, 50, 199] {
            assert_eq!(b.next_step(at(us)), IdleStep::Spin);
        }
        // Past the budget it parks, escalating 200, 400, 800 us and then capping.
        let sleeps: Vec<IdleStep> = (0..6).map(|i| b.next_step(at(300 + i))).collect();
        let us = |n: u64| IdleStep::Sleep(Duration::from_micros(n));
        assert_eq!(
            sleeps,
            [us(200), us(400), us(800), us(1_000), us(1_000), us(1_000)]
        );
        // A successful pop starts the next lull from a fresh spin.
        b.reset();
        assert_eq!(b.next_step(at(10_000)), IdleStep::Spin);
        assert_eq!(b.next_step(at(10_100)), IdleStep::Spin);
    }

    /// An idle consumer must not pin a core: it spins only for its budget, then
    /// parks. A pure spin would poll millions of times in this window.
    #[tokio::test(flavor = "current_thread")]
    async fn an_idle_consumer_parks_instead_of_spinning() {
        let mut b = IdleBackoff::new();
        let t0 = std::time::Instant::now();
        let mut polls = 0u64;
        while t0.elapsed() < std::time::Duration::from_millis(60) {
            b.wait().await;
            polls += 1;
        }
        println!("SET-C IDLE polls_in_60ms={polls} (a pure yield_now spin does tens of thousands)");
        assert!(
            polls < 5_000,
            "an idle consumer polled {polls} times in 60 ms"
        );
    }

    /// What a parked wake-up really costs on this machine (see `IdleBackoff`).
    #[tokio::test(flavor = "current_thread")]
    async fn a_parked_wakeup_is_measured_not_assumed() {
        let t = std::time::Instant::now();
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_micros(1_000)).await;
        }
        println!(
            "SET-C IDLE requested_sleep_us=1000 actual_avg_us={:.0}",
            t.elapsed().as_micros() as f64 / 10.0
        );
    }

    /// The Set C metric run: allocations per tick and tick-to-intent latency for
    /// the synchronous work of stages 2 and 3 (aggregate, build the world event,
    /// evaluate one strategy, and the tape frame). Printed with `--nocapture`.
    ///
    /// The `rkyv` decode of the trade payload is **not** in this loop: this crate
    /// does not depend on `rkyv`, so a test here cannot build an encoded tick. Its
    /// cost is measured where `rkyv` is available, in
    /// `crates/domain/tests/trade_decode_allocs.rs`.
    ///
    /// The allocation bound is asserted in every profile; the latency bound only
    /// in release, where the figure means something.
    #[test]
    fn per_tick_allocations_and_latency() {
        use strategy_runtime::runtime::StrategyInstance;
        use strategy_runtime::world::WorldEvent;

        let def: domain::strategy_def::StrategyDefinition =
            serde_json::from_value(serde_json::json!({
                "strategy_id": "metrics_ema", "definition_version": "1.0",
                "asset_class": "crypto_spot_cex", "min_trust_tier": "centralized_exchange",
                "inputs": [{ "lane": "features.technical", "instrument": "$bound_at_init",
                             "features": ["ema_7", "ema_21"] }],
                "nodes": [
                    { "id": "c1", "type": "condition",
                      "expr": "feature('ema_7') > feature('ema_21')" },
                    { "id": "s1", "type": "signal", "when": "c1", "emit": "long" }],
                "actions": [{ "on_signal": "long", "type": "place_order",
                    "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }]
            }))
            .unwrap();
        let mut inst = StrategyInstance::new(
            String::from("u"),
            String::from("BTC-USD"),
            def,
            chrono::Utc::now(),
        );
        let mut agg = Minute1Aggregator::new();
        let instrument_id = String::from("BTC-USD");
        let price = Price::from_str("50000.25").unwrap();
        let size = Size::from_str("0.01").unwrap();
        let bus = api::live_bus::channel();

        const N: usize = 20_000;
        let mut lat = Vec::with_capacity(N);
        let mut s2_allocs = 0u64;
        let mut s3_allocs = 0u64;
        let mut tape_allocs = 0u64;

        for i in 0..N as i64 {
            let ts_secs = 1_700_000_000 + i / 50;
            let t0 = std::time::Instant::now();
            let a0 = alloc_count::count();
            // Stage 2's synchronous work. With no socket listening the tape frame
            // is never formatted, so it is not on this path.
            let listening = api::live_bus::has_listeners(&bus);
            let _ = agg.on_trade(price.inner(), size.inner(), ts_secs);
            let bar = domain::payloads::bar::BarPayload::new(
                domain::payloads::bar::Timeframe::Minutes1,
                price,
                price,
                price,
                price,
                Size::from_decimal(Decimal::ONE),
                i as u64,
            );
            let event = WorldEvent::Bar {
                instrument_id: instrument_id.clone(),
                timeframe: domain::payloads::bar::Timeframe::Minutes1,
                bar,
                available_time: chrono::Utc::now(),
            };
            let a1 = alloc_count::count();
            // Stage 3.
            let out = inst.process_event(&event);
            let a2 = alloc_count::count();
            std::hint::black_box((&out, listening));
            lat.push(t0.elapsed().as_nanos() as u64);
            s2_allocs += a1 - a0;
            s3_allocs += a2 - a1;

            // The tape frame, as it is formatted only when a socket listens.
            let b0 = alloc_count::count();
            let frame = tape_frame_json(ts_secs * 1000, price, size, TradeSide::Buy);
            tape_allocs += alloc_count::count() - b0;
            std::hint::black_box(frame);
        }
        lat.sort_unstable();
        let p = |q: f64| lat[((N as f64 * q) as usize).min(N - 1)] as f64 / 1000.0;
        println!(
            "SET-C METRICS profile={} ticks={N} stage2_allocs_per_tick={:.2} stage3_allocs_per_tick={:.2} \
             tape_frame_allocs_per_tick_when_listening={:.2} tick_to_intent_us p50={:.1} p99={:.1} p999={:.1}",
            if cfg!(debug_assertions) { "debug" } else { "release" },
            s2_allocs as f64 / N as f64,
            s3_allocs as f64 / N as f64,
            tape_allocs as f64 / N as f64,
            p(0.50),
            p(0.99),
            p(0.999),
        );
        // Baseline in MASTER was ~150+ allocations per tick for the whole path.
        assert!(
            (s2_allocs + s3_allocs) as f64 / (N as f64) < 50.0,
            "per-tick allocations regressed past 50"
        );
        // With nobody listening, the tape costs nothing: one allocation is the
        // `Vec` of the formatted frame, and only when a socket is subscribed.
        assert!(
            tape_allocs as f64 / (N as f64) <= 2.0,
            "the tape frame allocates more than its one buffer"
        );
        #[cfg(not(debug_assertions))]
        assert!(
            p(0.99) < 500.0,
            "tick-to-intent p99 {} us exceeds 500 us",
            p(0.99)
        );
    }
}

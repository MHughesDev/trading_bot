//! In-process hot-path pipeline: socket-reader → bar-builder → strategy-eval → risk/exec.
//!
//! Each stage communicates via a bounded lock-free SPSC ring (rtrb).  JetStream
//! never appears on this path — it receives events via the async tee task.

use std::sync::Arc;

use backtest::CollectedBar;
use domain::{EventEnvelope, OrderIntent};
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
pub fn spawn_pipeline(
    symbol: String,
    instrument_id: String,
    asset_class: domain::instrument::AssetClass,
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
    // Paper engine fed by stage 2: every tick updates the per-instrument mark
    // board and fills any resting paper limit orders the mark crosses.
    paper_engine: Arc<PaperTradingEngine>,
    // Completed 1-minute bars are sent here for continuous ClickHouse persistence.
    bar_sink: BarSink,
) -> PipelineHandle {
    // Shared-data contract: this pipeline is the mark source for BOTH halves
    // (paper fills + live decisioning).  Registering the instrument's asset
    // class lets the paper half route orders to the right internal account.
    paper_engine.register_instrument(&instrument_id, asset_class);
    let (raw_prod, raw_cons) = rtrb::RingBuffer::<RawTick>::new(4096);
    let (world_prod, world_cons) = rtrb::RingBuffer::<WorldEvent>::new(1024);
    let (intent_prod, intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(256);

    let t1 = tokio::spawn(stage_socket_reader(symbol, raw_prod, tee_tx));
    let t2 = tokio::spawn(stage_bar_builder(
        instrument_id.clone(),
        raw_cons,
        world_prod,
        paper_engine,
        bar_sink,
    ));
    let t3 = tokio::spawn(stage_strategy_eval(
        instrument_id.clone(),
        world_cons,
        intent_prod,
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
async fn stage_socket_reader(
    symbol: String,
    raw_prod: rtrb::Producer<RawTick>,
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
) {
    info!(symbol, "hot-path stage 1 (socket-reader) starting");
    let collector = collectors::crypto::kraken::KrakenCollector::new(symbol.clone());
    if let Err(e) = collector.run_in_process(raw_prod, tee_tx).await {
        warn!(symbol, error = %e, "stage 1 (socket-reader) exited with error");
    }
}

/// Rolling 1-minute OHLCV bucket built from the live trade stream.
///
/// `bucket_start_secs` is the Unix second at the start of the open minute, or
/// `-1` when no bucket is open.  Open/high/low/close track the trade prices and
/// `volume` sums trade sizes; a bucket is finalized into a [`CollectedBar`] when
/// a trade for a later minute arrives or the wall clock passes the minute end.
struct Minute1Aggregator {
    bucket_start_secs: i64,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,
    trade_count: u64,
}

impl Minute1Aggregator {
    fn new() -> Self {
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
    fn on_trade(&mut self, price: Decimal, size: Decimal, ts_secs: i64) -> Option<CollectedBar> {
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

/// Stage 2: consume `RawTick`, feed the mark board, emit a per-tick `WorldEvent`
/// for live strategy evaluation, AND aggregate trades into 1-minute OHLCV bars
/// that are persisted to ClickHouse continuously (independent of any strategy).
async fn stage_bar_builder(
    instrument_id: String,
    mut raw_cons: rtrb::Consumer<RawTick>,
    mut world_prod: rtrb::Producer<WorldEvent>,
    paper_engine: Arc<PaperTradingEngine>,
    bar_sink: BarSink,
) {
    use domain::money::Size;
    use domain::payloads::bar::{BarPayload, Timeframe};
    use domain::payloads::trade::TradePayload;

    use chrono::DateTime;

    info!(instrument_id, "hot-path stage 2 (bar-builder) starting");

    let mut agg = Minute1Aggregator::new();
    let emit_bar = |bar: CollectedBar| {
        let _ = bar_sink.tx.send(PersistBar {
            instrument_id: instrument_id.clone(),
            venue_id: bar_sink.venue_id.clone(),
            source: bar_sink.source.clone(),
            trust_tier: bar_sink.trust_tier.clone(),
            bar,
        });
    };

    loop {
        match raw_cons.pop() {
            Ok(tick) => {
                let trade = match tick.decode_payload::<TradePayload>() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let price = trade.price;
                paper_engine.on_mark(&instrument_id, price);

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
                tokio::task::yield_now().await;
            }
        }
    }
}

/// Stage 3: consume `WorldEvent`, run strategy evaluation, push `OrderIntent`.
///
/// The strategy instance is a placeholder — real loading is wired through the
/// demand-manager / API layer.  The stage is structurally complete: it reads
/// `ring_world` and never awaits any network call.
async fn stage_strategy_eval(
    instrument_id: String,
    mut world_cons: rtrb::Consumer<WorldEvent>,
    mut intent_prod: rtrb::Producer<OrderIntent>,
) {
    use strategy_runtime::runtime::StrategyInstance;

    info!(instrument_id, "hot-path stage 3 (strategy-eval) starting");

    // No strategy loaded by default; placeholder for demand-manager integration.
    let mut strategy: Option<StrategyInstance> = None;

    loop {
        match world_cons.pop() {
            Ok(event) => {
                if let Some(ref mut instance) = strategy {
                    for intent in instance.process_event(&event) {
                        if intent_prod.push(intent).is_err() {
                            warn!(instrument_id, "ring_intent full — intent dropped");
                        }
                    }
                }
            }
            Err(_) => tokio::task::yield_now().await,
        }
    }
}

/// Stage 4: consume `OrderIntent`, apply risk gate, submit to execution engine.
async fn stage_risk_exec(
    instrument_id: String,
    mut intent_cons: rtrb::Consumer<OrderIntent>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
) {
    info!(instrument_id, "hot-path stage 4 (risk/exec) starting");
    loop {
        match intent_cons.pop() {
            Ok(intent) => {
                // Build a minimal GateContext from hot-path defaults.
                // Full position/price state wiring is addressed by set-C issue #5.
                let ctx = risk::GateContext::for_manual_order(
                    Decimal::ZERO,
                    None,
                    Decimal::ONE,
                    Decimal::ONE,
                    Decimal::ZERO,
                    true,
                    0,
                    0,
                );
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
            Err(_) => tokio::task::yield_now().await,
        }
    }
}

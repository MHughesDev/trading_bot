//! In-process hot-path pipeline: socket-reader → bar-builder → strategy-eval → risk/exec.
//!
//! Each stage communicates via a bounded lock-free SPSC ring (rtrb).  JetStream
//! never appears on this path — it receives events via the async tee task.

use std::sync::Arc;

use domain::{EventEnvelope, OrderIntent};
use rust_decimal::Decimal;
use strategy_runtime::world::WorldEvent;
use tracing::{info, warn};

/// A normalized trade tick from the socket reader.
pub type RawTick = EventEnvelope;

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
    tee_tx: tokio::sync::mpsc::UnboundedSender<RawTick>,
    execution_engine: Arc<execution::ExecutionEngine>,
    risk_gate: Arc<risk::RiskGate>,
) -> PipelineHandle {
    let (raw_prod, raw_cons) = rtrb::RingBuffer::<RawTick>::new(4096);
    let (world_prod, world_cons) = rtrb::RingBuffer::<WorldEvent>::new(1024);
    let (intent_prod, intent_cons) = rtrb::RingBuffer::<OrderIntent>::new(256);

    let t1 = tokio::spawn(stage_socket_reader(symbol, raw_prod, tee_tx));
    let t2 = tokio::spawn(stage_bar_builder(
        instrument_id.clone(),
        raw_cons,
        world_prod,
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

/// Stage 2: consume `RawTick`, emit `WorldEvent::Bar` per tick.
///
/// This is a minimal pass-through bar builder — each tick becomes a single-bar
/// snapshot so the strategy pipeline stays live.  A proper OHLCV aggregator
/// is addressed by set-C issues #3 and #24.
async fn stage_bar_builder(
    instrument_id: String,
    mut raw_cons: rtrb::Consumer<RawTick>,
    mut world_prod: rtrb::Producer<WorldEvent>,
) {
    use domain::money::Size;
    use domain::payloads::bar::{BarPayload, Timeframe};

    use chrono::DateTime;

    info!(instrument_id, "hot-path stage 2 (bar-builder) starting");
    loop {
        match raw_cons.pop() {
            Ok(tick) => {
                let price = match tick.decode_payload::<domain::payloads::trade::TradePayload>() {
                    Ok(p) => p.price,
                    Err(_) => continue,
                };
                let bar = BarPayload::new(
                    Timeframe::Minutes1,
                    price,
                    price,
                    price,
                    price,
                    Size::from_decimal(Decimal::ONE),
                    tick.sequence,
                );
                let secs = tick.timestamp_ns / 1_000_000_000;
                let nanos = (tick.timestamp_ns % 1_000_000_000).unsigned_abs() as u32;
                let available_time =
                    DateTime::from_timestamp(secs, nanos).unwrap_or_else(chrono::Utc::now);
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
            Err(_) => tokio::task::yield_now().await,
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
                    for intent in instance.process_event(event) {
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

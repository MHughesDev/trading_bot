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
    live: api::live_bus::LiveSender,
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

                // The tape. Published per tick so time & sales is genuinely live
                // rather than a minute behind the market.
                api::live_bus::publish(
                    &live,
                    domain::lanes::MARKET_TRADES,
                    &instrument_id,
                    serde_json::json!({
                        "ts": tick.timestamp_ns / 1_000_000,
                        "price": price.inner().to_string(),
                        "size": trade.size.inner().to_string(),
                        // An unknown aggressor is reported as such rather than
                        // guessed — the tape colours by side and a wrong colour
                        // is a wrong statement about who lifted the offer.
                        "side": match trade.side {
                            domain::payloads::trade::TradeSide::Buy => "buy",
                            domain::payloads::trade::TradeSide::Sell => "sell",
                            _ => "unknown",
                        },
                    }),
                );

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
            Err(_) => tokio::task::yield_now().await,
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

    let wanted: Vec<strategy_runtime::live::LiveKey> =
        declared.iter().map(|s| s.key()).collect();

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
    loop {
        match intent_cons.pop() {
            Ok(intent) => {
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
            Err(_) => tokio::task::yield_now().await,
        }
    }
}

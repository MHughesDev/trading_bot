"""
Live trading loop: Kraken WS → features → decision → risk → audit → optional QuestDB → execution.

**FB-CAN-029 canonical tick shape** (per message, when the watch gate runs a decision):

1. **Normalize / features** — Kraken WS → ``normalize_kraken_ws_message`` → bars + feature overlays.
2. **Shared decision step** — ``run_decision_tick`` (same as replay): pipeline canonical sequence then
   ``risk_engine.evaluate`` (see ``decision_engine/canonical_orchestrator.py`` + ``run_step.py``).
3. **Execution guidance + intent** — ``build_execution_context_from_decision`` / ``prepare_order_intent_for_execution``.
4. **Feedback** — ``apply_execution_feedback`` after a successful submit (stubbed fill quality).

Uses `run_decision_tick` (same path as `backtesting/replay.py`). Passes `feed_last_message_at` from WS.
"""

from __future__ import annotations

import asyncio
import logging
import os
import signal
import uuid
from datetime import UTC, datetime
from typing import Any

from decimal import Decimal

from app.config.settings import AppSettings, load_settings
from app.contracts.events import BarEvent
from app.contracts.execution_guidance import ExecutionFeedback
from app.contracts.risk import RiskState
from app.runtime.asset_lifecycle_state import effective_lifecycle_state
from app.runtime.asset_model_registry import list_symbols as list_asset_manifest_symbols
from app.runtime.live_watch_gate import record_decision_tick, should_run_decision_tick
from app.runtime.system_power import is_on, sync_from_disk
from data_plane.bars.rolling import RollingBars
from data_plane.features.pipeline import FeaturePipeline
from data_plane.ingest.news_ingest import aggregate_sentiment_for_symbols_async
from data_plane.memory.embeddings import feature_dict_to_embedding
from data_plane.memory.qdrant_memory import QdrantNewsMemory
from data_plane.memory.retrieval_loop import run_memory_retrieval_loop
from data_plane.ingest.kraken_normalizers import normalize_kraken_ws_message
from data_plane.ingest.kraken_rest import KrakenRESTClient
from data_plane.ingest.kraken_symbols import canonical_symbol_from_kraken_pair
from data_plane.ingest.kraken_ws import KrakenWebSocketClient
from data_plane.ingest.normalizers import OrderBookLevel2Snapshot, TickerSnapshot, TradeTick
from data_plane.ingest.product_cache import ProductMetadataCache
from data_plane.storage.questdb import QuestDBWriter
from data_plane.storage.startup_gap_detection import detect_canonical_bar_gaps
from orchestration.startup_canonical_backfill import run_startup_canonical_backfill
from decision_engine.audit import decision_trace
from decision_engine.feature_frame import enrich_bars_last_row, merge_feature_overlays
from decision_engine.features_live import feature_row_from_tick
from decision_engine.pipeline import DecisionPipeline
from decision_engine.run_step import run_decision_tick
from execution.adapters.base_adapter import PositionSnapshot
from execution.credentials import venue_credentials_configured
from data_plane.memory.execution_feedback_memory import update_execution_feedback_memory
from execution.execution_logic import (
    build_execution_context_from_decision,
    prepare_order_intent_for_execution,
)
from execution.service import ExecutionService
from execution.trade_markers import TradeMarker, append_marker
from risk_engine.engine import RiskEngine
from services.runtime_bridge import RuntimeHandoffBridge
from shared.messaging.factory import create_message_bus

logger = logging.getLogger(__name__)


def register_shutdown_signals(stop: asyncio.Event) -> None:
    """Register SIGINT/SIGTERM to set ``stop``. No-op on platforms without signal handlers."""

    def _sig(*_: Any) -> None:
        stop.set()

    loop = asyncio.get_running_loop()
    try:
        loop.add_signal_handler(signal.SIGINT, _sig)
        loop.add_signal_handler(signal.SIGTERM, _sig)
    except NotImplementedError:
        pass


def _positions_from_snapshots(
    snapshots: list[PositionSnapshot], symbols: list[str]
) -> dict[str, Decimal]:
    """Venue snapshot → per-symbol signed qty; missing symbols → 0."""
    by_sym = {s.symbol: s.quantity for s in snapshots}
    return {s: by_sym.get(s, Decimal(0)) for s in symbols}


def _infer_spread_bps(norm: Any) -> float:
    if isinstance(norm, TickerSnapshot) and norm.bid is not None and norm.ask is not None and norm.price:
        mid = float(norm.price)
        return (float(norm.ask) - float(norm.bid)) / mid * 10_000.0
    if isinstance(norm, OrderBookLevel2Snapshot) and norm.bids and norm.asks:
        bb = max(norm.bids, key=lambda x: x[0])[0]
        aa = min(norm.asks, key=lambda x: x[0])[0]
        mid = (bb + aa) / 2.0
        return (aa - bb) / mid * 10_000.0 if mid else 5.0
    return 5.0


def _tick_time(norm: Any) -> datetime:
    if isinstance(norm, (TickerSnapshot, TradeTick)):
        return norm.time if norm.time.tzinfo else norm.time.replace(tzinfo=UTC)
    return datetime.now(UTC)


async def _position_reconcile_loop(
    exec_svc: ExecutionService,
    positions: dict[str, Decimal],
    symbols: list[str],
    interval: float,
    stop: asyncio.Event,
) -> None:
    """Replace in-memory positions with venue truth (paper Alpaca → configured symbols)."""
    while not stop.is_set():
        try:
            await asyncio.wait_for(stop.wait(), timeout=interval)
            return
        except TimeoutError:
            pass
        try:
            snaps = await exec_svc.adapter.fetch_positions()
            merged = _positions_from_snapshots(snaps, symbols)
            positions.clear()
            positions.update(merged)
            logger.info("position_reconcile %s", {k: str(v) for k, v in positions.items()})
        except Exception:
            logger.exception(
                "position_reconcile failed symbols=%s",
                ",".join(symbols),
            )


async def _sentiment_refresh_loop(
    sentiment: dict[str, float],
    symbols: list[str],
    settings: AppSettings,
    interval: float,
    stop: asyncio.Event,
) -> None:
    try:
        agg = await aggregate_sentiment_for_symbols_async(
            symbols,
            use_finbert=settings.sentiment_use_finbert,
            rss_feeds=settings.news_rss_feeds,
            fetch_timeout_seconds=settings.news_fetch_timeout_seconds,
        )
        sentiment.clear()
        sentiment.update(agg)
    except Exception:
        logger.exception("initial sentiment aggregate failed symbols=%s", ",".join(symbols))
    while not stop.is_set():
        try:
            await asyncio.wait_for(stop.wait(), timeout=interval)
            return
        except TimeoutError:
            pass
        try:
            agg = await aggregate_sentiment_for_symbols_async(
                symbols,
                use_finbert=settings.sentiment_use_finbert,
                rss_feeds=settings.news_rss_feeds,
                fetch_timeout_seconds=settings.news_fetch_timeout_seconds,
            )
            sentiment.clear()
            sentiment.update(agg)
        except Exception:
            logger.exception("sentiment refresh failed symbols=%s", ",".join(symbols))


async def run_live_loop(
    *,
    symbols: list[str] | None = None,
    max_iterations: int | None = None,
    settings: AppSettings | None = None,
    stop_event: asyncio.Event | None = None,
) -> None:
    cfg = settings or load_settings()
    syms = symbols or cfg.market_data_symbols
    ws = KrakenWebSocketClient(syms)
    feature_pipeline = FeaturePipeline(
        return_windows=cfg.features_return_windows,
        volatility_windows=cfg.features_volatility_windows,
    )
    pipeline = DecisionPipeline(settings=cfg)
    risk_engine = RiskEngine(cfg)
    exec_svc = ExecutionService(cfg)
    risk_state = RiskState()
    exec_feedback: dict[str, dict[str, float]] = {}
    venue_creds_ok = venue_credentials_configured(cfg)
    if not venue_creds_ok:
        logger.warning(
            "venue API credentials not set for execution_mode=%s — order submission disabled "
            "(set NM_COINBASE_* for live or NM_ALPACA_* for paper, or use Streamlit venue onboarding)",
            cfg.execution_mode,
        )
    mem_by_symbol: dict[str, dict[str, float]] = {s: {} for s in syms}
    sentiment_overlay: dict[str, float] = {}
    last_feature_row: dict[str, dict[str, float]] = {s: {} for s in syms}
    stop = stop_event or asyncio.Event()

    runtime_bridge: RuntimeHandoffBridge | None = None
    use_external_execution_gateway = False
    if cfg.microservices_runtime_bridge_enabled:
        try:
            if cfg.microservices_execution_gateway_mode == "external":
                os.environ["NM_MESSAGING_BACKEND"] = "redis_streams"
                os.environ["NM_REDIS_URL"] = cfg.redis_url
                use_external_execution_gateway = True
            runtime_bridge = RuntimeHandoffBridge(
                create_message_bus(),
                execution_gateway_mode=cfg.microservices_execution_gateway_mode,
            )
            logger.info(
                "microservice runtime bridge enabled (mode=%s)",
                cfg.microservices_execution_gateway_mode,
            )
        except Exception:
            logger.exception("microservice runtime bridge init failed; continuing without bridge")
            runtime_bridge = None
            use_external_execution_gateway = False

    qdb: QuestDBWriter | None = None
    if (
        cfg.questdb_persist_decision_traces
        or cfg.questdb_persist_canonical_bars
        or cfg.questdb_startup_gap_detection
        or cfg.questdb_startup_kraken_backfill
    ):
        qdb = QuestDBWriter(
            cfg.questdb_host,
            cfg.questdb_port,
            cfg.questdb_user,
            cfg.questdb_password,
            cfg.questdb_database,
            batch_max_rows=cfg.questdb_batch_max_rows,
        )
        await qdb.connect()

    bar_sec = max(1, int(cfg.market_data_bar_interval_seconds))
    if qdb is not None and (
        cfg.questdb_startup_gap_detection or cfg.questdb_startup_kraken_backfill
    ):
        try:
            init_syms = list_asset_manifest_symbols()
            if init_syms:
                gaps = await detect_canonical_bar_gaps(
                    qdb,
                    symbols=init_syms,
                    interval_seconds=bar_sec,
                )
                if cfg.questdb_startup_gap_detection:
                    for g in gaps:
                        if g.gap_detected:
                            logger.warning(
                                "canonical_bar_gap %s",
                                g.to_log_dict(),
                            )
                        else:
                            logger.info(
                                "canonical_bar_gap_ok %s",
                                g.to_log_dict(),
                            )
                if cfg.questdb_startup_kraken_backfill:
                    await run_startup_canonical_backfill(cfg, qdb, gaps=gaps)
        except Exception:
            logger.exception("startup canonical bar gap detection / backfill failed")

    rest_client: KrakenRESTClient | None = None
    product_cache: ProductMetadataCache | None = None
    try:
        rest_client = KrakenRESTClient()
        product_cache = ProductMetadataCache(rest_client)
        await product_cache.refresh_if_stale()
    except Exception:
        logger.exception("product metadata cache failed — tradable defaults to True")
        product_cache = None

    sentiment_stop = asyncio.Event()
    qdrant_mem: QdrantNewsMemory | None = None
    try:
        qdrant_mem = QdrantNewsMemory(cfg.qdrant_url, cfg.memory_qdrant_collection)
    except Exception:
        logger.warning("Qdrant client unavailable; memory features default to neutral")

    def _on_mem(sym: str):
        def _merge(mapped: dict[str, float]) -> None:
            mem_by_symbol[sym] = mapped

        return _merge

    mem_tasks: list[asyncio.Task[None]] = []
    if qdrant_mem is not None:
        for sym in syms:
            mem_tasks.append(
                asyncio.create_task(
                    run_memory_retrieval_loop(
                        cfg,
                        sym,
                        _on_mem(sym),
                        query_embedding_fn=lambda s=sym: feature_dict_to_embedding(
                            last_feature_row.get(s, {}), dim=qdrant_mem.vector_size
                        ),
                        memory=qdrant_mem,
                    )
                )
            )
    sentiment_task = asyncio.create_task(
        _sentiment_refresh_loop(
            sentiment_overlay,
            syms,
            cfg,
            float(cfg.memory_retrieval_interval_seconds),
            sentiment_stop,
        )
    )

    qdb_flush_stop = asyncio.Event()
    qdb_flush_task: asyncio.Task[None] | None = None
    if (
        qdb is not None
        and cfg.questdb_flush_interval_seconds > 0
        and cfg.questdb_persist_decision_traces
    ):
        async def _flush_loop() -> None:
            while not qdb_flush_stop.is_set():
                try:
                    await asyncio.wait_for(
                        qdb_flush_stop.wait(),
                        timeout=float(cfg.questdb_flush_interval_seconds),
                    )
                    return
                except TimeoutError:
                    pass
                try:
                    await qdb.flush_decision_traces()
                except Exception:
                    logger.exception("questdb periodic flush failed")

        qdb_flush_task = asyncio.create_task(_flush_loop())

    rollers: dict[str, RollingBars] = {
        s: RollingBars(s, interval_seconds=bar_sec) for s in syms
    }
    positions: dict[str, Decimal] = {s: Decimal(0) for s in syms}
    last_decision_monotonic: dict[str, float] = {}

    reconcile_stop: asyncio.Event | None = None
    reconcile_task: asyncio.Task[None] | None = None
    if cfg.execution_mode == "paper" and cfg.position_reconcile_enabled:
        try:
            snaps = await exec_svc.adapter.fetch_positions()
            positions.update(_positions_from_snapshots(snaps, syms))
            logger.info("initial positions from venue %s", {k: str(v) for k, v in positions.items()})
        except Exception:
            logger.exception("initial position fetch failed; using zeros until reconcile")
        reconcile_stop = asyncio.Event()
        reconcile_task = asyncio.create_task(
            _position_reconcile_loop(
                exec_svc,
                positions,
                syms,
                float(cfg.position_reconcile_interval_seconds),
                reconcile_stop,
            )
        )

    n = 0
    try:
        async for msg in ws.stream_messages():
            if stop.is_set():
                break
            sync_from_disk()
            if not is_on():
                logger.info("system power off — exiting live loop")
                break
            norm = normalize_kraken_ws_message(msg)
            if norm is None:
                continue
            ws_pair = getattr(norm, "symbol", None)
            if not ws_pair:
                continue
            symbol = canonical_symbol_from_kraken_pair(str(ws_pair))
            if symbol not in rollers:
                continue

            px = float(getattr(norm, "price", 0.0) or 0.0)
            ts = _tick_time(norm)
            sz = float(getattr(norm, "size", 0.0) or 0.0) if isinstance(norm, TradeTick) else 0.0
            completed_bar = rollers[symbol].on_tick(px, ts, sz)
            if (
                qdb is not None
                and cfg.questdb_persist_canonical_bars
                and completed_bar is not None
            ):
                try:
                    bar = BarEvent(
                        timestamp=completed_bar["timestamp"],
                        symbol=symbol,
                        open=float(completed_bar["open"]),
                        high=float(completed_bar["high"]),
                        low=float(completed_bar["low"]),
                        close=float(completed_bar["close"]),
                        volume=float(completed_bar["volume"]),
                        interval_seconds=bar_sec,
                        source="kraken",
                        schema_version=1,
                    )
                    await qdb.insert_bar(bar)
                except Exception:
                    logger.exception("questdb insert_bar failed")

            overlay = feature_row_from_tick(
                norm,
                memory=mem_by_symbol.get(symbol),
                sentiment=sentiment_overlay or None,
                pipeline=feature_pipeline,
            )
            last_feature_row[symbol] = overlay
            bar_df = rollers[symbol].bars_frame_with_partial()
            bar_feats = enrich_bars_last_row(bar_df, feature_pipeline)
            if bar_feats:
                feats = merge_feature_overlays(bar_feats, overlay)
            else:
                feats = overlay
            spread_bps = _infer_spread_bps(norm)
            data_ts = ts
            tradable = product_cache.is_tradable(symbol) if product_cache else True

            mid = float(feats.get("close", px)) or px or 1.0
            run_decision = should_run_decision_tick(
                symbol,
                cfg,
                effective_lifecycle=effective_lifecycle_state,
                last_decision_monotonic=last_decision_monotonic,
            )
            if run_decision:
                regime, fc, route, proposal, trade, risk_state = run_decision_tick(
                    symbol=symbol,
                    feature_row=feats,
                    spread_bps=spread_bps,
                    risk_state=risk_state,
                    pipeline=pipeline,
                    risk_engine=risk_engine,
                    mid_price=mid,
                    data_timestamp=data_ts,
                    feed_last_message_at=ws.last_message_at,
                    product_tradable=tradable,
                    position_signed_qty=positions.get(symbol, Decimal(0)),
                    portfolio_equity_usd=risk_engine.current_equity,
                    execution_feedback_state=exec_feedback,
                )
                record_decision_tick(symbol, last_decision_monotonic)
            else:
                logger.debug(
                    "decision_tick_skipped symbol=%s lifecycle_gate=%s min_interval_s=%s",
                    symbol,
                    cfg.live_watch_lifecycle_gate,
                    cfg.live_decision_min_interval_seconds,
                )
                continue

            if runtime_bridge is not None:
                try:
                    direction = int(proposal.direction) if proposal is not None else 0
                    size_fraction = float(proposal.size_fraction) if proposal is not None else 0.0
                    runtime_bridge.process_feature_row(
                        {
                            "symbol": symbol,
                            "direction": direction,
                            "size_fraction": size_fraction,
                            "route_id": proposal.route_id.value if proposal is not None else "NO_TRADE",
                        }
                    )
                except Exception:
                    logger.exception("runtime bridge shadow handoff failed")

            oid = str(uuid.uuid4())
            intent = None
            if trade:
                raw = risk_engine.to_order_intent(trade, sign=False)
                fb = exec_feedback.get(symbol)
                xctx = build_execution_context_from_decision(
                    spread_bps=spread_bps,
                    feature_row=feats,
                    regime=regime,
                    forecast=fc,
                    risk=risk_state,
                    mid_price=mid,
                    forecast_packet=pipeline.last_forecast_packet,
                    execution_feedback_bucket=fb,
                )
                meta = dict(raw.metadata or {})
                meta["execution_context"] = xctx
                raw = raw.model_copy(update={"metadata": meta})
                intent = prepare_order_intent_for_execution(raw, cfg)
            exec_blocked = trade is not None and intent is None
            trace = decision_trace(
                symbol=symbol,
                regime=regime,
                forecast=fc,
                route=route,
                proposal=proposal,
                risk=risk_state,
                trade_allowed=trade is not None and not exec_blocked,
                order_intent=intent,
                block_reason="execution_guidance_suppress"
                if exec_blocked
                else (None if trade else "risk_blocked_or_no_trade"),
                correlation_id=oid,
                forecast_packet=pipeline.last_forecast_packet,
            )
            logger.info("decision_trace %s", trace)
            if qdb:
                try:
                    await qdb.insert_decision_trace_dict(trace)
                except Exception:
                    logger.exception("questdb insert_decision_trace failed")

            if trade and intent:
                if runtime_bridge is not None and use_external_execution_gateway:
                    logger.info(
                        "external execution gateway: skipping in-process submit_order for %s",
                        symbol,
                    )
                elif not venue_creds_ok:
                    pass
                else:
                    try:
                        ack = await exec_svc.submit_order(intent)
                        exec_feedback[symbol] = update_execution_feedback_memory(
                            symbol,
                            ExecutionFeedback(
                                fill_ratio=1.0,
                                realized_slippage_bps=0.0,
                                fill_latency_ms=35.0,
                                venue_quality_score=0.85,
                                adapter=str(getattr(ack, "adapter", "")),
                            ),
                            state=exec_feedback,
                        )
                        try:
                            append_marker(
                                TradeMarker(
                                    ts=datetime.now(UTC),
                                    symbol=str(intent.symbol),
                                    side=intent.side.value,
                                    quantity=str(intent.quantity),
                                    source="intent_submit",
                                    correlation_id=oid,
                                    execution_mode=cfg.execution_mode,
                                )
                            )
                        except Exception:
                            logger.exception("trade marker append failed")
                        q = Decimal(str(trade.quantity))
                        if trade.side == "buy":
                            positions[symbol] = positions.get(symbol, Decimal(0)) + q
                        else:
                            positions[symbol] = positions.get(symbol, Decimal(0)) - q
                    except Exception:
                        logger.exception("submit_order failed")

            n += 1
            if max_iterations is not None and n >= max_iterations:
                break
    finally:
        qdb_flush_stop.set()
        if qdb_flush_task is not None:
            qdb_flush_task.cancel()
            try:
                await qdb_flush_task
            except asyncio.CancelledError:
                pass
        if reconcile_stop is not None:
            reconcile_stop.set()
        if reconcile_task is not None:
            reconcile_task.cancel()
            try:
                await reconcile_task
            except asyncio.CancelledError:
                pass
        sentiment_stop.set()
        for t in mem_tasks:
            t.cancel()
        for t in mem_tasks:
            try:
                await t
            except asyncio.CancelledError:
                pass
        sentiment_task.cancel()
        try:
            await sentiment_task
        except asyncio.CancelledError:
            pass
        if qdb:
            await qdb.aclose()
        if rest_client:
            await rest_client.aclose()


def main() -> None:
    """Run until SIGINT/SIGTERM (Unix) or use max_iterations in tests."""
    logging.basicConfig(level=logging.INFO)

    async def _run() -> None:
        stop = asyncio.Event()
        register_shutdown_signals(stop)
        await run_live_loop(stop_event=stop)

    asyncio.run(_run())


if __name__ == "__main__":
    main()

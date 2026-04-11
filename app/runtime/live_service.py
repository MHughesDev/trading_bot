"""
Live trading loop: Coinbase WS → features → decision → risk → audit → optional QuestDB → execution.

Uses `run_decision_tick` (same path as `backtesting/replay.py`). Passes `feed_last_message_at` from WS.
"""

from __future__ import annotations

import asyncio
import logging
import signal
import uuid
from datetime import UTC, datetime
from typing import Any

from decimal import Decimal

from app.config.settings import AppSettings, load_settings
from app.contracts.risk import RiskState
from data_plane.bars.rolling import RollingMinuteBars
from data_plane.features.pipeline import FeaturePipeline
from data_plane.ingest.news_ingest import aggregate_sentiment_for_symbols_async
from data_plane.memory.embeddings import feature_dict_to_embedding
from data_plane.memory.qdrant_memory import QdrantNewsMemory
from data_plane.memory.retrieval_loop import run_memory_retrieval_loop
from data_plane.ingest.coinbase_rest import CoinbaseRESTClient
from data_plane.ingest.coinbase_ws import CoinbaseWebSocketClient
from data_plane.ingest.normalizers import OrderBookLevel2Snapshot, TickerSnapshot, TradeTick, normalize_ws_message
from data_plane.ingest.product_cache import ProductMetadataCache
from data_plane.storage.questdb import QuestDBWriter
from decision_engine.audit import decision_trace
from decision_engine.feature_frame import enrich_bars_last_row, merge_feature_overlays
from decision_engine.features_live import feature_row_from_tick
from decision_engine.pipeline import DecisionPipeline
from decision_engine.run_step import run_decision_tick
from execution.adapters.base_adapter import PositionSnapshot
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine

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
    """Replace in-memory positions with venue truth (paper Alpaca → Coinbase product ids)."""
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
            logger.exception("position_reconcile failed")


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
        logger.exception("initial sentiment aggregate failed")
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
            logger.exception("sentiment refresh failed")


async def run_live_loop(
    *,
    symbols: list[str] | None = None,
    max_iterations: int | None = None,
    settings: AppSettings | None = None,
    stop_event: asyncio.Event | None = None,
) -> None:
    cfg = settings or load_settings()
    syms = symbols or cfg.market_data_symbols
    ws = CoinbaseWebSocketClient(syms)
    feature_pipeline = FeaturePipeline(
        return_windows=cfg.features_return_windows,
        volatility_windows=cfg.features_volatility_windows,
    )
    pipeline = DecisionPipeline(settings=cfg)
    risk_engine = RiskEngine(cfg)
    exec_svc = ExecutionService(cfg)
    risk_state = RiskState()
    mem_by_symbol: dict[str, dict[str, float]] = {s: {} for s in syms}
    sentiment_overlay: dict[str, float] = {}
    last_feature_row: dict[str, dict[str, float]] = {s: {} for s in syms}
    stop = stop_event or asyncio.Event()

    qdb: QuestDBWriter | None = None
    if cfg.questdb_persist_decision_traces:
        qdb = QuestDBWriter(
            cfg.questdb_host,
            cfg.questdb_port,
            cfg.questdb_user,
            cfg.questdb_password,
            cfg.questdb_database,
            batch_max_rows=cfg.questdb_batch_max_rows,
        )
        await qdb.connect()

    rest_client: CoinbaseRESTClient | None = None
    product_cache: ProductMetadataCache | None = None
    try:
        rest_client = CoinbaseRESTClient()
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
    if qdb is not None and cfg.questdb_flush_interval_seconds > 0:
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

    rollers: dict[str, RollingMinuteBars] = {s: RollingMinuteBars(s) for s in syms}
    positions: dict[str, Decimal] = {s: Decimal(0) for s in syms}

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
            norm = normalize_ws_message(msg)
            if norm is None:
                continue
            symbol = getattr(norm, "symbol", None)
            if not symbol:
                continue

            px = float(getattr(norm, "price", 0.0) or 0.0)
            ts = _tick_time(norm)
            sz = float(getattr(norm, "size", 0.0) or 0.0) if isinstance(norm, TradeTick) else 0.0
            rollers[symbol].on_tick(px, ts, sz)

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
            )

            oid = str(uuid.uuid4())
            intent = risk_engine.to_order_intent(trade) if trade else None
            trace = decision_trace(
                symbol=symbol,
                regime=regime,
                forecast=fc,
                route=route,
                proposal=proposal,
                risk=risk_state,
                trade_allowed=trade is not None,
                order_intent=intent,
                block_reason=None if trade else "risk_blocked_or_no_trade",
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
                try:
                    await exec_svc.submit_order(intent)
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

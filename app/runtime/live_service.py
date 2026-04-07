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

from app.config.settings import AppSettings, load_settings
from app.contracts.risk import RiskState
from data_plane.features.pipeline import FeaturePipeline
from data_plane.ingest.coinbase_rest import CoinbaseRESTClient
from data_plane.ingest.coinbase_ws import CoinbaseWebSocketClient
from data_plane.ingest.normalizers import OrderBookLevel2Snapshot, TickerSnapshot, TradeTick, normalize_ws_message
from data_plane.ingest.product_cache import ProductMetadataCache
from data_plane.storage.questdb import QuestDBWriter
from decision_engine.audit import decision_trace
from decision_engine.features_live import feature_row_from_tick
from decision_engine.pipeline import DecisionPipeline
from decision_engine.run_step import run_decision_tick
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine

logger = logging.getLogger(__name__)


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


async def _memory_tick_loop(
    mem: dict[str, float],
    interval: float,
    stop: asyncio.Event,
) -> None:
    """Placeholder 60s memory features until Qdrant encoder is wired."""
    while not stop.is_set():
        try:
            await asyncio.wait_for(stop.wait(), timeout=interval)
            return
        except TimeoutError:
            mem["mem_sim_mean"] = 0.0
            mem["mem_sent_mean"] = 0.0
            mem["mem_shock"] = 0.0


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
    mem: dict[str, float] = {}
    stop = stop_event or asyncio.Event()

    qdb: QuestDBWriter | None = None
    if cfg.questdb_persist_decision_traces:
        qdb = QuestDBWriter(
            cfg.questdb_host,
            cfg.questdb_port,
            cfg.questdb_user,
            cfg.questdb_password,
            cfg.questdb_database,
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

    mem_stop = asyncio.Event()
    mem_task = asyncio.create_task(
        _memory_tick_loop(mem, float(cfg.memory_retrieval_interval_seconds), mem_stop)
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

            feats = feature_row_from_tick(norm, memory=mem, pipeline=feature_pipeline)
            spread_bps = _infer_spread_bps(norm)
            data_ts = _tick_time(norm)
            tradable = product_cache.is_tradable(symbol) if product_cache else True

            regime, fc, route, proposal, trade, risk_state = run_decision_tick(
                symbol=symbol,
                feature_row=feats,
                spread_bps=spread_bps,
                risk_state=risk_state,
                pipeline=pipeline,
                risk_engine=risk_engine,
                mid_price=float(feats.get("close", 1.0)) or 1.0,
                data_timestamp=data_ts,
                feed_last_message_at=ws.last_message_at,
                product_tradable=tradable,
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
                except Exception:
                    logger.exception("submit_order failed")

            n += 1
            if max_iterations is not None and n >= max_iterations:
                break
    finally:
        mem_stop.set()
        mem_task.cancel()
        try:
            await mem_task
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

        def _sig(*_: Any) -> None:
            stop.set()

        loop = asyncio.get_running_loop()
        try:
            loop.add_signal_handler(signal.SIGINT, _sig)
            loop.add_signal_handler(signal.SIGTERM, _sig)
        except NotImplementedError:
            pass

        await run_live_loop(stop_event=stop)

    asyncio.run(_run())


if __name__ == "__main__":
    main()

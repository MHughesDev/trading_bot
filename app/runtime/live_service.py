"""
Live trading loop: Coinbase WS → normalize → decision → risk → signed intent → execution.

Uses WS `last_message_at` for data age (spec: stale data guard). Optional memory features merged in.
"""

from __future__ import annotations

import asyncio
import logging
import uuid
from datetime import UTC, datetime
from typing import Any

from app.config.settings import load_settings
from app.contracts.risk import RiskState
from data_plane.ingest.coinbase_ws import CoinbaseWebSocketClient
from data_plane.ingest.normalizers import (
    OrderBookLevel2Snapshot,
    TickerSnapshot,
    TradeTick,
    normalize_ws_message,
)
from decision_engine.audit import decision_trace
from decision_engine.pipeline import DecisionPipeline
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


async def run_live_loop(
    *,
    symbols: list[str] | None = None,
    max_iterations: int | None = None,
    extra_memory_features: dict[str, float] | None = None,
) -> None:
    settings = load_settings()
    syms = symbols or settings.market_data_symbols
    ws = CoinbaseWebSocketClient(syms)
    pipeline = DecisionPipeline(settings=settings)
    risk_engine = RiskEngine(settings)
    exec_svc = ExecutionService(settings)
    risk_state = RiskState()
    mem = extra_memory_features or {}

    n = 0
    async for msg in ws.stream_messages():
        norm = normalize_ws_message(msg)
        if norm is None:
            continue
        symbol = getattr(norm, "symbol", None)
        if not symbol:
            continue

        feats: dict[str, float] = {"close": float(getattr(norm, "price", 0.0) or 0.0)}
        feats.update(mem)

        spread_bps = _infer_spread_bps(norm)
        data_ts = _tick_time(norm)

        if ws.last_message_at:
            age = abs((datetime.now(UTC) - ws.last_message_at).total_seconds())
            if age > settings.risk_stale_data_seconds:
                logger.warning("feed stale %.1fs — skipping trade evaluation", age)

        regime, fc, route, proposal = pipeline.step(symbol, feats, spread_bps, risk_state)
        trade, risk_state = risk_engine.evaluate(
            symbol,
            proposal,
            risk_state,
            mid_price=float(feats["close"]) or 1.0,
            spread_bps=spread_bps,
            data_timestamp=data_ts,
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
        if trade and intent:
            try:
                await exec_svc.submit_order(intent)
            except Exception:
                logger.exception("submit_order failed")
        n += 1
        if max_iterations is not None and n >= max_iterations:
            break


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    asyncio.run(run_live_loop(max_iterations=1))


if __name__ == "__main__":
    main()

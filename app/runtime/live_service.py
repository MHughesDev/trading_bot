"""
Live trading loop skeleton: Coinbase WS → normalize → decision → risk → signed intent → execution.

Wire QuestDB/Redis/Qdrant and bar aggregation in follow-up PRs; this enforces pipeline order.
"""

from __future__ import annotations

import asyncio
import logging
import uuid
from datetime import UTC, datetime

from app.config.settings import load_settings
from app.contracts.risk import RiskState
from data_plane.ingest.coinbase_ws import CoinbaseWebSocketClient
from data_plane.ingest.normalizers import normalize_ws_message
from decision_engine.audit import decision_trace
from decision_engine.pipeline import DecisionPipeline
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine

logger = logging.getLogger(__name__)


async def run_live_loop(
    *,
    symbols: list[str] | None = None,
    max_iterations: int | None = None,
) -> None:
    settings = load_settings()
    syms = symbols or settings.market_data_symbols
    ws = CoinbaseWebSocketClient(syms)
    pipeline = DecisionPipeline()
    risk_engine = RiskEngine(settings)
    exec_svc = ExecutionService(settings)
    risk_state = RiskState()

    n = 0
    async for msg in ws.stream_messages():
        norm = normalize_ws_message(msg)
        if norm is None:
            continue
        symbol = getattr(norm, "symbol", None)
        if not symbol:
            continue
        feats = {"close": float(getattr(norm, "price", 0.0) or 0.0)}
        spread_bps = 5.0
        regime, fc, route, proposal = pipeline.step(symbol, feats, spread_bps, risk_state)
        trade, risk_state = risk_engine.evaluate(
            symbol,
            proposal,
            risk_state,
            mid_price=float(feats["close"]) or 1.0,
            spread_bps=spread_bps,
            data_timestamp=datetime.now(UTC),
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

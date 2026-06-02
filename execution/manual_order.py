"""Submit an explicit (human-operator or AI-agent) order via the risk + execution path.

A manual order carries a caller-specified quantity (no automated sizing) but still passes
the :class:`~risk_engine.engine.RiskEngine` hard gates and is risk-signed before reaching a
venue adapter — the same final authority as automated trades. Used by the control-plane
``/trade/*`` endpoints (human UI) and the MCP server (AI agent) so both share one audited
action surface (human-first platform; AI acts through the same gate).
"""

from __future__ import annotations

import asyncio
import logging
from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from app.contracts.orders import OrderSide
from app.contracts.risk import RiskState
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine

logger = logging.getLogger(__name__)


async def submit_manual_order(
    settings: AppSettings,
    symbol: str,
    side: str,
    quantity: Decimal,
    *,
    order_type: str = "market",
    limit_price: Decimal | None = None,
    mid_price: float | None = None,
    available_cash_usd: float | None = None,
    risk_state: RiskState | None = None,
    source: str = "manual",
    execution_service: ExecutionService | None = None,
) -> dict[str, Any]:
    """Risk-gate, sign, and submit a single explicit order. Returns a JSON-serializable summary."""
    sym = symbol.strip()
    try:
        order_side = OrderSide(str(side).strip().lower())
    except ValueError:
        return {
            "submitted": False,
            "error": f"invalid side {side!r}",
            "blocked": ["invalid_side"],
            "acks": [],
        }

    svc = execution_service or ExecutionService(settings)
    adapter = svc.adapter_for_symbol(sym)

    pos_qty = Decimal(0)
    try:
        snaps = await adapter.fetch_positions()
        for s in snaps:
            if getattr(s, "symbol", None) == sym:
                pos_qty = s.quantity
                break
    except Exception as e:  # position unknown — proceed without reduce/cash context
        logger.warning("manual_order fetch_positions failed symbol=%s: %s", sym, e)

    risk = RiskEngine(settings)
    ta, rs = risk.evaluate_manual_order(
        sym,
        side=order_side,
        quantity=Decimal(str(quantity)),
        risk=risk_state or RiskState(),
        mid_price=float(mid_price) if mid_price is not None else 0.0,
        position_signed_qty=pos_qty,
        available_cash_usd=available_cash_usd,
        order_type=order_type,
        limit_price=Decimal(str(limit_price)) if limit_price is not None else None,
    )
    if ta is None:
        return {
            "submitted": False,
            "error": "risk_blocked",
            "blocked": list(rs.last_risk_block_codes or []),
            "acks": [],
        }

    intent = risk.to_order_intent(ta, sign=False)
    intent = intent.model_copy(
        update={
            "metadata": {
                **intent.metadata,
                "manual_order": True,
                "source": source,
                # Skip forecast-based execution-guidance suppression: this is a deliberate
                # caller order. RiskEngine gates + risk-signing still apply via submit_order.
                "skip_execution_guidance": True,
            }
        }
    )
    try:
        ack = await svc.submit_order(intent)
    except Exception as e:
        logger.exception("manual_order submit failed symbol=%s", sym)
        return {"submitted": False, "error": str(e), "blocked": [], "acks": []}

    return {
        "submitted": True,
        "error": None,
        "blocked": [],
        "side": order_side.value,
        "quantity": str(ta.quantity),
        "order_type": ta.order_type,
        "position_qty_before": str(pos_qty),
        "acks": [ack.model_dump(mode="json")],
    }


def submit_manual_order_sync(
    settings: AppSettings,
    symbol: str,
    side: str,
    quantity: Decimal,
    **kwargs: Any,
) -> dict[str, Any]:
    """Run :func:`submit_manual_order` from a sync context (e.g. a FastAPI sync route)."""
    return asyncio.run(submit_manual_order(settings, symbol, side, quantity, **kwargs))

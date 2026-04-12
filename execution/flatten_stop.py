"""Flatten open venue position for one symbol (FB-AP-032 — per-asset Stop)."""

from __future__ import annotations

import asyncio
import logging
from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from app.contracts.risk import RiskState, SystemMode
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine
from risk_engine.signing import sign_order_intent

logger = logging.getLogger(__name__)


def _sign_flatten_intent(settings: AppSettings, intent: OrderIntent) -> OrderIntent:
    secret = (
        settings.risk_signing_secret.get_secret_value() if settings.risk_signing_secret else None
    )
    if secret:
        return sign_order_intent(intent, secret)
    return intent


async def flatten_symbol_position(
    settings: AppSettings,
    symbol: str,
    *,
    execution_service: ExecutionService | None = None,
) -> dict[str, Any]:
    """
    If the venue has a non-zero position for ``symbol``, submit **market** order(s) to close.

    Uses ``ExecutionService.adapter_for_symbol`` (per-asset paper/live). For **long**: one sell
    of ``abs(qty)``. For **short** (negative qty): one buy of ``abs(qty)``. Partial fills are not
    re-polled here — venue may leave residual; operator may Stop again or reconcile manually.

    Returns a JSON-serializable summary (``submitted``, ``skipped``, ``acks``, ``error``).
    """
    sym = symbol.strip()
    svc = execution_service or ExecutionService(settings)
    adapter = svc.adapter_for_symbol(sym)

    try:
        snaps = await adapter.fetch_positions()
    except Exception as e:
        logger.warning("flatten fetch_positions failed symbol=%s: %s", sym, e)
        return {
            "submitted": False,
            "skipped": "fetch_positions_failed",
            "error": str(e),
            "acks": [],
            "lifecycle_continue": False,
        }

    pos_qty = Decimal(0)
    for s in snaps:
        if getattr(s, "symbol", None) == sym:
            pos_qty = s.quantity
            break

    if pos_qty == 0:
        return {
            "submitted": False,
            "skipped": "flat",
            "error": None,
            "acks": [],
            "lifecycle_continue": True,
        }

    risk = RiskEngine(settings)
    ta, _ = risk.evaluate(
        sym,
        None,
        RiskState(mode=SystemMode.FLATTEN_ALL),
        mid_price=1.0,
        spread_bps=0.0,
        data_timestamp=None,
        position_signed_qty=pos_qty,
    )
    if ta is None:
        return {
            "submitted": False,
            "skipped": "risk_blocked",
            "error": None,
            "acks": [],
            "lifecycle_continue": False,
        }

    intent = risk.to_order_intent(ta)
    # Ensure metadata marks operator flatten (signing uses canonical payload)
    intent = intent.model_copy(
        update={
            "metadata": {
                **intent.metadata,
                "flatten_stop": True,
                "reason": "asset_lifecycle_stop",
            }
        }
    )
    intent = _sign_flatten_intent(settings, intent)

    try:
        ack = await svc.submit_order(intent)
    except Exception as e:
        logger.exception("flatten submit_order failed symbol=%s", sym)
        return {
            "submitted": False,
            "skipped": None,
            "error": str(e),
            "acks": [],
            "lifecycle_continue": False,
        }

    return {
        "submitted": True,
        "skipped": None,
        "error": None,
        "position_qty_before": str(pos_qty),
        "acks": [ack.model_dump(mode="json")],
        "lifecycle_continue": True,
    }


def flatten_symbol_position_sync(
    settings: AppSettings,
    symbol: str,
    *,
    execution_service: ExecutionService | None = None,
) -> dict[str, Any]:
    """Run :func:`flatten_symbol_position` from a sync context (e.g. FastAPI sync route)."""
    return asyncio.run(
        flatten_symbol_position(settings, symbol, execution_service=execution_service)
    )

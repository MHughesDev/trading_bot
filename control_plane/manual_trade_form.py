"""Pure helpers for the asset-page manual-trade form.

No Streamlit import on purpose: the body-building + validation logic is unit-testable here
and mirrors the ``ManualOrderRequest`` contract, so the UI cannot build a request the backend
would reject. ``asset_page._render_trade_panel`` renders the widgets and calls these.
"""

from __future__ import annotations

from typing import Any

ORDER_TYPES = ("market", "limit", "stop", "stop_limit")
TIME_IN_FORCE = ("gtc", "ioc", "fok", "gtd")


def order_needs_limit_price(order_type: str) -> bool:
    return order_type in ("limit", "stop_limit")


def order_needs_stop_price(order_type: str) -> bool:
    return order_type in ("stop", "stop_limit")


def build_manual_order_body(
    *,
    symbol: str,
    side: str,
    quantity: float,
    order_type: str = "market",
    limit_price: float | None = None,
    stop_price: float | None = None,
    time_in_force: str = "gtc",
) -> dict[str, Any]:
    """Build the ``POST /trade/order`` body, omitting prices irrelevant to ``order_type``.

    Raises ``ValueError`` for a non-positive quantity or a missing/non-positive required price
    (same rules the ``ManualOrderRequest`` contract enforces), so the UI can give immediate
    feedback before calling the API.
    """
    if order_type not in ORDER_TYPES:
        raise ValueError(f"order_type must be one of {ORDER_TYPES}")
    if time_in_force not in TIME_IN_FORCE:
        raise ValueError(f"time_in_force must be one of {TIME_IN_FORCE}")
    if quantity is None or float(quantity) <= 0:
        raise ValueError("quantity must be greater than 0")

    body: dict[str, Any] = {
        "symbol": symbol,
        "side": side,
        "quantity": str(quantity),
        "order_type": order_type,
        "time_in_force": time_in_force,
    }
    if order_needs_limit_price(order_type):
        if not limit_price or float(limit_price) <= 0:
            raise ValueError("limit price is required for limit and stop-limit orders")
        body["limit_price"] = str(limit_price)
    if order_needs_stop_price(order_type):
        if not stop_price or float(stop_price) <= 0:
            raise ValueError("stop price is required for stop and stop-limit orders")
        body["stop_price"] = str(stop_price)
    return body

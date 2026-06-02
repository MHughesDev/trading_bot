"""Asset-page manual-trade form helpers: body building + validation match the backend contract."""

from __future__ import annotations

import pytest

from app.contracts.manual_order import ManualOrderRequest
from control_plane.manual_trade_form import (
    build_manual_order_body,
    order_needs_limit_price,
    order_needs_stop_price,
)


def test_price_requirement_predicates() -> None:
    assert order_needs_limit_price("limit") and order_needs_limit_price("stop_limit")
    assert not order_needs_limit_price("market") and not order_needs_limit_price("stop")
    assert order_needs_stop_price("stop") and order_needs_stop_price("stop_limit")
    assert not order_needs_stop_price("market") and not order_needs_stop_price("limit")


def test_market_body_accepted_by_contract() -> None:
    body = build_manual_order_body(symbol="BTC-USD", side="buy", quantity=0.01)
    assert body["order_type"] == "market" and body["time_in_force"] == "gtc"
    assert "limit_price" not in body and "stop_price" not in body
    ManualOrderRequest(**body)  # contract accepts it


def test_limit_and_stop_limit_bodies_accepted() -> None:
    limit = build_manual_order_body(
        symbol="BTC-USD", side="buy", quantity=0.01, order_type="limit",
        limit_price=65000.0, time_in_force="ioc",
    )
    assert limit["limit_price"] == "65000.0" and limit["time_in_force"] == "ioc"
    ManualOrderRequest(**limit)

    sl = build_manual_order_body(
        symbol="BTC-USD", side="sell", quantity=0.01, order_type="stop_limit",
        limit_price=59950.0, stop_price=60000.0,
    )
    assert sl["limit_price"] == "59950.0" and sl["stop_price"] == "60000.0"
    ManualOrderRequest(**sl)


def test_stop_body_accepted() -> None:
    body = build_manual_order_body(
        symbol="BTC-USD", side="sell", quantity=0.01, order_type="stop", stop_price=60000.0
    )
    assert body["stop_price"] == "60000.0" and "limit_price" not in body
    ManualOrderRequest(**body)


@pytest.mark.parametrize(
    "kwargs",
    [
        {"order_type": "limit"},  # missing limit_price
        {"order_type": "stop"},  # missing stop_price
        {"order_type": "stop_limit", "limit_price": 1.0},  # missing stop_price
        {"order_type": "stop_limit", "stop_price": 1.0},  # missing limit_price
    ],
)
def test_missing_required_price_raises(kwargs: dict) -> None:
    with pytest.raises(ValueError):
        build_manual_order_body(symbol="BTC-USD", side="buy", quantity=0.01, **kwargs)


def test_nonpositive_quantity_and_bad_enums_raise() -> None:
    with pytest.raises(ValueError):
        build_manual_order_body(symbol="BTC-USD", side="buy", quantity=0.0)
    with pytest.raises(ValueError):
        build_manual_order_body(symbol="BTC-USD", side="buy", quantity=1.0, order_type="iceberg")
    with pytest.raises(ValueError):
        build_manual_order_body(
            symbol="BTC-USD", side="buy", quantity=1.0, time_in_force="forever"
        )

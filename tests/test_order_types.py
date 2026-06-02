"""Limit/stop/stop-limit order-type support across contracts, risk, and venue adapters.

Market-only was the prior limitation (Alpaca stripped non-market; Coinbase raised). These
tests pin the type/price/TIF mapping for both venues plus the risk + contract plumbing.
"""

from __future__ import annotations

from decimal import Decimal

import pytest

from app.config.settings import AppSettings
from app.contracts.manual_order import ManualOrderRequest
from app.contracts.orders import OrderIntent, OrderSide, OrderType, TimeInForce
from app.contracts.risk import RiskState
from execution.adapters.alpaca_paper import alpaca_request_plan
from execution.coinbase_advanced_http import build_coinbase_order_configuration
from risk_engine.engine import RiskEngine


def _intent(order_type: OrderType, **kw) -> OrderIntent:
    base = dict(symbol="BTC-USD", side=OrderSide.BUY, quantity=Decimal("0.01"))
    base.update(kw)
    return OrderIntent(order_type=order_type, **base)


# --- Alpaca request plan (pure mapping; no alpaca-py import needed) ---


def test_alpaca_plan_market() -> None:
    cls, kw = alpaca_request_plan(_intent(OrderType.MARKET))
    assert cls == "MarketOrderRequest"
    assert kw["qty"] == 0.01 and kw["side"] == "buy" and kw["time_in_force"] == "gtc"
    assert "limit_price" not in kw and "stop_price" not in kw


def test_alpaca_plan_limit_carries_price_and_tif() -> None:
    cls, kw = alpaca_request_plan(
        _intent(OrderType.LIMIT, limit_price=Decimal("65000"), time_in_force=TimeInForce.IOC)
    )
    assert cls == "LimitOrderRequest"
    assert kw["limit_price"] == 65000.0 and kw["time_in_force"] == "ioc"


def test_alpaca_plan_stop_and_stop_limit() -> None:
    cls, kw = alpaca_request_plan(_intent(OrderType.STOP, stop_price=Decimal("60000")))
    assert cls == "StopOrderRequest" and kw["stop_price"] == 60000.0
    cls2, kw2 = alpaca_request_plan(
        _intent(OrderType.STOP_LIMIT, stop_price=Decimal("60000"), limit_price=Decimal("59950"))
    )
    assert cls2 == "StopLimitOrderRequest"
    assert kw2["stop_price"] == 60000.0 and kw2["limit_price"] == 59950.0


def test_alpaca_plan_gtd_falls_back_to_gtc() -> None:
    _cls, kw = alpaca_request_plan(_intent(OrderType.MARKET, time_in_force=TimeInForce.GTD))
    assert kw["time_in_force"] == "gtc"


@pytest.mark.parametrize(
    "ot,kw",
    [
        (OrderType.LIMIT, {}),
        (OrderType.STOP, {}),
        (OrderType.STOP_LIMIT, {"stop_price": Decimal("1")}),
        (OrderType.STOP_LIMIT, {"limit_price": Decimal("1")}),
    ],
)
def test_alpaca_plan_missing_price_raises(ot: OrderType, kw: dict) -> None:
    with pytest.raises(ValueError):
        alpaca_request_plan(_intent(ot, **kw))


# --- Coinbase order_configuration (pure mapping; no network) ---


def test_coinbase_market_config() -> None:
    cfg = build_coinbase_order_configuration(_intent(OrderType.MARKET))
    assert cfg == {"market_market_ioc": {"base_size": "0.01"}}


def test_coinbase_limit_gtc_and_fok() -> None:
    gtc = build_coinbase_order_configuration(_intent(OrderType.LIMIT, limit_price=Decimal("100")))
    assert gtc["limit_limit_gtc"]["limit_price"] == "100"
    fok = build_coinbase_order_configuration(
        _intent(OrderType.LIMIT, limit_price=Decimal("100"), time_in_force=TimeInForce.FOK)
    )
    assert "limit_limit_fok" in fok


def test_coinbase_stop_limit_direction_by_side() -> None:
    buy = build_coinbase_order_configuration(
        _intent(OrderType.STOP_LIMIT, stop_price=Decimal("110"), limit_price=Decimal("111"))
    )
    assert buy["stop_limit_stop_limit_gtc"]["stop_direction"] == "STOP_DIRECTION_STOP_UP"
    sell = build_coinbase_order_configuration(
        OrderIntent(
            symbol="BTC-USD",
            side=OrderSide.SELL,
            quantity=Decimal("0.01"),
            order_type=OrderType.STOP_LIMIT,
            stop_price=Decimal("90"),
            limit_price=Decimal("89"),
        )
    )
    assert sell["stop_limit_stop_limit_gtc"]["stop_direction"] == "STOP_DIRECTION_STOP_DOWN"


def test_coinbase_stop_market_is_rejected_not_faked() -> None:
    # Coinbase has no native stop-market; we reject rather than silently downgrade to market.
    with pytest.raises(NotImplementedError):
        build_coinbase_order_configuration(_intent(OrderType.STOP, stop_price=Decimal("60000")))


def test_coinbase_limit_missing_price_raises() -> None:
    with pytest.raises(ValueError):
        build_coinbase_order_configuration(_intent(OrderType.LIMIT))


# --- Risk engine manual path: pass-through of type/price/TIF ---


def test_risk_manual_order_passes_stop_limit_through() -> None:
    risk = RiskEngine(AppSettings())
    ta, _rs = risk.evaluate_manual_order(
        "BTC-USD",
        side=OrderSide.BUY,
        quantity=Decimal("0.01"),
        risk=RiskState(),
        order_type="stop_limit",
        limit_price=Decimal("101"),
        stop_price=Decimal("100"),
        time_in_force="ioc",
    )
    assert ta is not None
    assert ta.order_type == "stop_limit"
    assert ta.limit_price == Decimal("101") and ta.stop_price == Decimal("100")
    assert ta.time_in_force == "ioc"

    intent = risk.to_order_intent(ta, sign=False)
    assert intent.order_type == OrderType.STOP_LIMIT
    assert intent.limit_price == Decimal("101") and intent.stop_price == Decimal("100")
    assert intent.time_in_force == TimeInForce.IOC


def test_risk_manual_order_unknown_type_falls_back_to_market() -> None:
    risk = RiskEngine(AppSettings())
    ta, _rs = risk.evaluate_manual_order(
        "BTC-USD",
        side=OrderSide.SELL,
        quantity=Decimal("0.01"),
        risk=RiskState(),
        order_type="banana",
    )
    assert ta is not None and ta.order_type == "market"


# --- Contract validation: incomplete limit/stop orders are rejected up front ---


def test_manual_request_requires_limit_price() -> None:
    with pytest.raises(ValueError):
        ManualOrderRequest(symbol="BTC-USD", side="buy", quantity=Decimal("1"), order_type="limit")


def test_manual_request_requires_stop_price() -> None:
    with pytest.raises(ValueError):
        ManualOrderRequest(
            symbol="BTC-USD",
            side="buy",
            quantity=Decimal("1"),
            order_type="stop_limit",
            limit_price=Decimal("100"),
        )


def test_manual_request_rejects_bad_type_and_tif() -> None:
    with pytest.raises(ValueError):
        ManualOrderRequest(symbol="BTC-USD", side="buy", quantity=Decimal("1"), order_type="iceberg")
    with pytest.raises(ValueError):
        ManualOrderRequest(
            symbol="BTC-USD", side="buy", quantity=Decimal("1"), time_in_force="eternal"
        )


def test_manual_request_market_still_valid() -> None:
    req = ManualOrderRequest(symbol="BTC-USD", side="buy", quantity=Decimal("1"))
    assert req.order_type == "market" and req.time_in_force == "gtc"


# --- End-to-end HTTP path through the control plane (mock adapter) ---


def test_trade_order_limit_submits_over_http(monkeypatch: pytest.MonkeyPatch) -> None:
    from fastapi.testclient import TestClient

    from control_plane import api

    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(control_plane_api_key=None, execution_mode="paper", auth_session_enabled=False),
    )
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "mock_alpaca_paper")
    client = TestClient(api.app)

    ok = client.post(
        "/trade/order",
        json={
            "symbol": "BTC-USD",
            "side": "buy",
            "quantity": "0.01",
            "order_type": "limit",
            "limit_price": "65000",
            "time_in_force": "ioc",
        },
    )
    assert ok.status_code == 200 and ok.json()["submitted"] is True
    assert ok.json()["order_type"] == "limit"

    # A limit order without a price is rejected by contract validation (422) before risk.
    bad = client.post(
        "/trade/order",
        json={"symbol": "BTC-USD", "side": "buy", "quantity": "0.01", "order_type": "limit"},
    )
    assert bad.status_code == 422

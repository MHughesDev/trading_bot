import pytest

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent, OrderSide, OrderType
from execution.intent_gate import execution_allowed
from risk_engine.signing import sign_order_intent, verify_order_intent


def test_order_intent_rejects_raw_text_metadata():
    with pytest.raises(ValueError, match="raw news"):
        OrderIntent(
            symbol="BTC-USD",
            side=OrderSide.BUY,
            quantity=1,
            order_type=OrderType.MARKET,
            metadata={"headline": "buy now"},
        )


def test_sign_and_verify_roundtrip():
    intent = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=1,
        order_type=OrderType.MARKET,
        metadata={"route_id": "SCALPING"},
    )
    signed = sign_order_intent(intent, "test-secret")
    assert verify_order_intent(signed, "test-secret")
    assert not verify_order_intent(signed, "wrong")


def test_execution_gate_requires_signature_when_configured():
    settings = AppSettings(
        risk_signing_secret="secret",
        allow_unsigned_execution=False,
    )
    raw = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=1,
        order_type=OrderType.MARKET,
    )
    assert not execution_allowed(raw, settings)
    assert execution_allowed(sign_order_intent(raw, "secret"), settings)


def test_execution_gate_allows_unsigned_when_no_secret():
    settings = AppSettings(risk_signing_secret=None, allow_unsigned_execution=False)
    raw = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=1,
        order_type=OrderType.MARKET,
    )
    assert execution_allowed(raw, settings)

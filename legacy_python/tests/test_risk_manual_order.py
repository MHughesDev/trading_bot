"""Tests for RiskEngine.evaluate_manual_order (explicit human/agent order gating)."""

from __future__ import annotations

from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.orders import OrderSide
from app.contracts.risk import RiskState, SystemMode
from risk_engine.engine import (
    RISK_BLOCK_AVAILABLE_CASH,
    RISK_BLOCK_DRAWDOWN,
    RISK_BLOCK_MAINTENANCE,
    RISK_BLOCK_PAUSE_NEW_ENTRIES,
    RISK_BLOCK_PRODUCT_UNTRADABLE,
    RISK_BLOCK_QTY_ZERO,
    RISK_BLOCK_REDUCE_ONLY_ADD,
    RISK_BLOCK_SPREAD_WIDE,
    RiskEngine,
)


def _engine() -> RiskEngine:
    return RiskEngine(AppSettings())


def test_manual_buy_passes_with_explicit_quantity() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("0.25"), risk=RiskState()
    )
    assert ta is not None
    assert ta.side == "buy"
    assert ta.quantity == Decimal("0.25")  # honored as given, no sizing
    assert not rs.last_risk_block_codes


def test_manual_zero_quantity_blocked() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("0"), risk=RiskState()
    )
    assert ta is None
    assert RISK_BLOCK_QTY_ZERO in rs.last_risk_block_codes


def test_manual_product_untradable_blocked() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"), risk=RiskState(), product_tradable=False
    )
    assert ta is None
    assert RISK_BLOCK_PRODUCT_UNTRADABLE in rs.last_risk_block_codes


def test_manual_maintenance_blocks() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.SELL, quantity=Decimal("1"),
        risk=RiskState(mode=SystemMode.MAINTENANCE),
    )
    assert ta is None
    assert RISK_BLOCK_MAINTENANCE in rs.last_risk_block_codes


def test_manual_pause_blocks_increase_allows_reduce() -> None:
    eng = _engine()
    # Increasing (open from flat) blocked under PAUSE_NEW_ENTRIES.
    ta, rs = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"),
        risk=RiskState(mode=SystemMode.PAUSE_NEW_ENTRIES),
    )
    assert ta is None
    assert RISK_BLOCK_PAUSE_NEW_ENTRIES in rs.last_risk_block_codes
    # Reducing a long position is allowed even when paused.
    ta2, _ = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.SELL, quantity=Decimal("0.5"),
        risk=RiskState(mode=SystemMode.PAUSE_NEW_ENTRIES),
        position_signed_qty=Decimal("1"),
    )
    assert ta2 is not None
    assert ta2.side == "sell"


def test_manual_reduce_only_blocks_add_and_clamps_reduce() -> None:
    eng = _engine()
    ta, rs = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"),
        risk=RiskState(mode=SystemMode.REDUCE_ONLY), position_signed_qty=Decimal("1"),
    )
    assert ta is None
    assert RISK_BLOCK_REDUCE_ONLY_ADD in rs.last_risk_block_codes
    # Oversized reduce clamps to the open position size.
    ta2, _ = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.SELL, quantity=Decimal("5"),
        risk=RiskState(mode=SystemMode.REDUCE_ONLY), position_signed_qty=Decimal("1"),
    )
    assert ta2 is not None
    assert ta2.quantity == Decimal("1")


def test_manual_available_cash_gate() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("2"), risk=RiskState(),
        mid_price=100.0, available_cash_usd=100.0,
    )
    assert ta is None
    assert RISK_BLOCK_AVAILABLE_CASH in rs.last_risk_block_codes


def test_manual_drawdown_blocks_increase_only() -> None:
    eng = _engine()
    eng.update_equity(40_000.0)  # 60% drawdown from the 100k peak
    ta, rs = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"), risk=RiskState()
    )
    assert ta is None
    assert RISK_BLOCK_DRAWDOWN in rs.last_risk_block_codes
    # A reducing sell is still allowed so positions can be exited during drawdown.
    ta2, _ = eng.evaluate_manual_order(
        "BTC-USD", side=OrderSide.SELL, quantity=Decimal("1"), risk=RiskState(),
        position_signed_qty=Decimal("1"),
    )
    assert ta2 is not None


def test_manual_wide_spread_blocks_increase() -> None:
    ta, rs = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"), risk=RiskState(),
        spread_bps=99999.0,
    )
    assert ta is None
    assert RISK_BLOCK_SPREAD_WIDE in rs.last_risk_block_codes


def test_manual_limit_order_carries_price() -> None:
    ta, _ = _engine().evaluate_manual_order(
        "BTC-USD", side=OrderSide.BUY, quantity=Decimal("1"), risk=RiskState(),
        order_type="limit", limit_price=Decimal("95000"),
    )
    assert ta is not None
    assert ta.order_type == "limit"
    assert ta.limit_price == Decimal("95000")

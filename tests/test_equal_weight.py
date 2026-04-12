"""risk_engine.equal_weight (FB-N3 reference)."""

from __future__ import annotations

from decimal import Decimal

from risk_engine.equal_weight import equal_weight_fractions


def test_equal_weight_three_symbols() -> None:
    w = equal_weight_fractions(3)
    assert len(w) == 3
    assert sum(w) == Decimal(1)
    mn, mx = min(w), max(w)
    assert mx - mn < Decimal("1e-18")


def test_equal_weight_zero() -> None:
    assert equal_weight_fractions(0) == []

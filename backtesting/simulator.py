"""Simulated fills with slippage for backtests."""

from __future__ import annotations

from decimal import Decimal


def apply_slippage(price: float, side: str, slippage_bps: float) -> float:
    adj = slippage_bps / 10_000.0 * price
    if side == "buy":
        return price + adj
    return price - adj


def simulated_fill_notional(price: float, qty: Decimal, side: str, slippage_bps: float) -> Decimal:
    px = apply_slippage(price, side, slippage_bps)
    return Decimal(str(px)) * qty

"""Simulated fills: slippage (optional seeded noise) and fees for backtests."""

from __future__ import annotations

import random
from decimal import Decimal


def apply_slippage(price: float, side: str, slippage_bps: float) -> float:
    """Deterministic half-spread style slippage (no RNG)."""
    adj = slippage_bps / 10_000.0 * price
    if side == "buy":
        return price + adj
    return price - adj


def effective_slippage_bps(base_bps: float, noise_bps: float, rng: random.Random) -> float:
    """Base slippage plus uniform noise in [-noise_bps, +noise_bps] (0 if noise_bps <= 0)."""
    if noise_bps <= 0:
        return base_bps
    return base_bps + rng.uniform(-noise_bps, noise_bps)


def fill_price_with_slippage(
    mid_price: float,
    side: str,
    *,
    slippage_bps: float,
    slippage_noise_bps: float = 0.0,
    rng: random.Random | None = None,
) -> float:
    """Execution price after slippage; uses `rng` when noise is positive."""
    r = rng or random.Random()
    bps = effective_slippage_bps(slippage_bps, slippage_noise_bps, r)
    return apply_slippage(mid_price, side, bps)


def fee_on_notional(notional: Decimal, fee_bps: float) -> Decimal:
    return abs(notional) * Decimal(str(fee_bps)) / Decimal("10000")


def simulated_fill_notional(price: float, qty: Decimal, side: str, slippage_bps: float) -> Decimal:
    px = apply_slippage(price, side, slippage_bps)
    return Decimal(str(px)) * qty


def cash_delta_for_trade(
    *,
    side: str,
    qty: Decimal,
    fill_price: float,
    fee_bps: float,
) -> tuple[Decimal, Decimal]:
    """
    Return (cash_delta, fee_paid).

    Buy: cash decreases by notional + fee. Sell: cash increases by notional - fee.
    """
    notional = qty * Decimal(str(fill_price))
    fee = fee_on_notional(notional, fee_bps)
    if side == "buy":
        return (-notional - fee, fee)
    return (notional - fee, fee)


def make_replay_rng(seed: int | None) -> random.Random:
    """Isolated RNG for replay so global `random` state is untouched."""
    return random.Random(seed)

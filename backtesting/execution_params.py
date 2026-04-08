"""Parameters for simulated fills in replay (fees, slippage, RNG) — aligned with AppSettings."""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal

from app.config.settings import AppSettings


@dataclass(frozen=True)
class BacktestExecutionParams:
    """Slippage and fees for `replay_decisions` when `track_portfolio` is enabled."""

    slippage_bps: float
    fee_bps: float
    slippage_noise_bps: float = 0.0
    rng_seed: int | None = None
    initial_cash: Decimal = Decimal("100000")

    @classmethod
    def from_settings(cls, s: AppSettings) -> BacktestExecutionParams:
        return cls(
            slippage_bps=s.backtesting_slippage_bps,
            fee_bps=s.backtesting_fee_bps,
            slippage_noise_bps=s.backtesting_slippage_noise_bps,
            rng_seed=s.backtesting_rng_seed,
            initial_cash=Decimal(str(s.backtesting_initial_cash_usd)),
        )

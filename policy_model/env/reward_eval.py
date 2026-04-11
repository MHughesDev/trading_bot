"""Replay-based reward calibration and simple regime stress (FB-PL-PG4)."""

from __future__ import annotations

from collections.abc import Sequence

import numpy as np

from policy_model.env.reward import one_step_reward


def evaluate_reward_weights(
    delta_log_equities: Sequence[float],
    turnovers: Sequence[float],
    costs: Sequence[float],
    *,
    lam_turn: float = 0.01,
    lam_cost: float = 1.0,
) -> float:
    """Mean one-step reward over a batch (for tuning λ weights offline)."""
    vals = []
    for de, to, co in zip(delta_log_equities, turnovers, costs, strict=True):
        vals.append(one_step_reward(de, to, co, lam_turn=lam_turn, lam_cost=lam_cost))
    return float(np.mean(vals)) if vals else 0.0


def stress_regime_returns(base_return: float, stress_sigma: float = 0.05, n: int = 100) -> float:
    """Monte Carlo: mean reward when returns are perturbed (stress harness)."""
    rng = np.random.default_rng(0)
    rs = base_return + rng.normal(0, stress_sigma, size=n)
    return float(np.mean([one_step_reward(float(r), 0.1, 0.0001) for r in rs]))

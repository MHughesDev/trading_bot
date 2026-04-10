"""Canonical decomposed reward (human policy spec §6, §17)."""

from __future__ import annotations


def one_step_reward(
    delta_log_equity: float,
    turnover: float,
    cost: float,
    *,
    lam_turn: float = 0.01,
    lam_cost: float = 1.0,
    lam_dd: float = 0.0,
    drawdown_increment: float = 0.0,
    lam_inv: float = 0.0,
    inventory_penalty: float = 0.0,
    lam_churn: float = 0.0,
    action_churn: float = 0.0,
) -> float:
    return (
        delta_log_equity
        - lam_turn * turnover
        - lam_cost * cost
        - lam_dd * drawdown_increment
        - lam_inv * inventory_penalty
        - lam_churn * action_churn
    )

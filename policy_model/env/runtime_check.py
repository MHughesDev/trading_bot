"""Runtime checks that concrete envs satisfy `TradingPolicyEnvironment` (FB-PL-P0-01)."""

from __future__ import annotations

from typing import TypeVar

from policy_model.env.environment import TradingPolicyEnvironment

T = TypeVar("T", bound=TradingPolicyEnvironment)


def assert_trading_policy_environment(env: T) -> T:
    """
    Validate at runtime that `env` implements the protocol (structural subtyping).

    Raises `TypeError` if `issubclass` / `isinstance` checks fail.
    """
    if not isinstance(env, TradingPolicyEnvironment):
        raise TypeError(f"{type(env).__name__} does not satisfy TradingPolicyEnvironment")
    return env

"""Tests for the strategy catalogue (FB-AP-XXX) — must work without `nautilus_trader` installed."""

from __future__ import annotations

import pytest

from strategies import get_strategy, list_strategies
from strategies.registry import StrategyDescriptor, StrategyParam, _import_from_path, register


def test_list_strategies_includes_ema_cross() -> None:
    keys = {d.key for d in list_strategies()}
    assert "ema_cross" in keys


def test_get_strategy_returns_descriptor_or_none() -> None:
    desc = get_strategy("ema_cross")
    assert isinstance(desc, StrategyDescriptor)
    assert desc.name == "EMA Cross"
    assert get_strategy("does_not_exist") is None


def test_default_params_matches_param_defaults() -> None:
    desc = get_strategy("ema_cross")
    assert desc is not None
    defaults = desc.default_params()
    assert defaults == {p.name: p.default for p in desc.params}
    assert defaults["fast_ema_period"] == 10
    assert defaults["slow_ema_period"] == 20
    assert defaults["trade_size"] == "0.01"


def test_duplicate_registration_rejected() -> None:
    descriptor = StrategyDescriptor(
        key="ema_cross",
        name="dup",
        description="dup",
        strategy_path="strategies.ema_cross_strategy:EMACrossStrategy",
        config_path="strategies.ema_cross_strategy:EMACrossStrategyConfig",
    )
    with pytest.raises(ValueError, match="duplicate strategy key"):
        register(descriptor)


def test_import_from_path_requires_module_and_class() -> None:
    with pytest.raises(ValueError, match="invalid importable path"):
        _import_from_path("not-a-path")


def test_import_from_path_resolves_real_class() -> None:
    cls = _import_from_path("strategies.registry:StrategyDescriptor")
    assert cls is StrategyDescriptor


def test_import_from_path_missing_attribute_raises() -> None:
    with pytest.raises(AttributeError, match="has no attribute"):
        _import_from_path("strategies.registry:DoesNotExist")


def test_strategy_param_is_frozen() -> None:
    param = StrategyParam(name="x", kind="int", default=1)
    with pytest.raises(Exception):
        param.default = 2  # type: ignore[misc]

"""Lightweight, import-free catalogue of backtestable strategies.

:class:`StrategyDescriptor` carries everything the control plane / Asset-page UI needs to list
and configure a strategy (name, description, tunable-parameter schema) *without* importing
``nautilus_trader`` or the strategy module itself — listing strategies must work even when the
optional ``backtest_nautilus`` extra isn't installed. The actual ``Strategy``/``StrategyConfig``
classes are resolved lazily, on demand, by :func:`get_strategy`.
"""

from __future__ import annotations

import importlib
from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class StrategyParam:
    """One tunable strategy parameter, described for UI rendering."""

    name: str
    kind: str  # "int" | "float" | "decimal" | "string"
    default: Any
    description: str = ""
    minimum: float | None = None
    maximum: float | None = None


@dataclass(frozen=True)
class StrategyDescriptor:
    """Catalogue entry: metadata + import paths for one strategy, resolved lazily."""

    key: str
    name: str
    description: str
    strategy_path: str  # "module.path:ClassName" — importable Strategy subclass
    config_path: str  # "module.path:ConfigClassName" — importable StrategyConfig subclass
    params: tuple[StrategyParam, ...] = field(default_factory=tuple)

    def default_params(self) -> dict[str, Any]:
        return {p.name: p.default for p in self.params}

    def load(self) -> tuple[type, type]:
        """Import and return ``(StrategyClass, StrategyConfigClass)``.

        Raises :class:`ImportError` (with a clear message) if ``nautilus_trader`` — or its
        drop-in fork — isn't installed; raises :class:`AttributeError` if the configured
        path doesn't resolve to the expected class.
        """
        strategy_cls = _import_from_path(self.strategy_path)
        config_cls = _import_from_path(self.config_path)
        return strategy_cls, config_cls


def _import_from_path(path: str) -> type:
    module_path, _, class_name = path.partition(":")
    if not module_path or not class_name:
        raise ValueError(f"invalid importable path (want 'module:ClassName'): {path!r}")
    try:
        module = importlib.import_module(module_path)
    except ModuleNotFoundError as exc:
        if exc.name and exc.name.startswith("nautilus_trader"):
            raise ImportError(
                "Backtesting requires the `nautilus_trader` package (or the platform's "
                'fork): pip install -e ".[backtest_nautilus]" — see strategies/README.md'
            ) from exc
        raise
    try:
        return getattr(module, class_name)
    except AttributeError as exc:
        raise AttributeError(f"{module_path!r} has no attribute {class_name!r}") from exc


_REGISTRY: dict[str, StrategyDescriptor] = {}


def register(descriptor: StrategyDescriptor) -> StrategyDescriptor:
    if descriptor.key in _REGISTRY:
        raise ValueError(f"duplicate strategy key: {descriptor.key}")
    _REGISTRY[descriptor.key] = descriptor
    return descriptor


def list_strategies() -> list[StrategyDescriptor]:
    """All registered strategies, in registration order."""
    return list(_REGISTRY.values())


def get_strategy(key: str) -> StrategyDescriptor | None:
    return _REGISTRY.get(key)


register(
    StrategyDescriptor(
        key="ema_cross",
        name="EMA Cross",
        description=(
            "Dual exponential-moving-average crossover. Goes long when the fast EMA crosses "
            "above the slow EMA, short on the reverse cross, flat otherwise. Reference "
            "strategy — no alpha claim."
        ),
        strategy_path="strategies.ema_cross_strategy:EMACrossStrategy",
        config_path="strategies.ema_cross_strategy:EMACrossStrategyConfig",
        params=(
            StrategyParam(
                name="fast_ema_period",
                kind="int",
                default=10,
                description="Fast EMA lookback (bars)",
                minimum=2,
                maximum=200,
            ),
            StrategyParam(
                name="slow_ema_period",
                kind="int",
                default=20,
                description="Slow EMA lookback (bars); must exceed fast_ema_period",
                minimum=3,
                maximum=400,
            ),
            StrategyParam(
                name="trade_size",
                kind="decimal",
                default="0.01",
                description="Position size per entry, in base-asset units",
            ),
        ),
    )
)

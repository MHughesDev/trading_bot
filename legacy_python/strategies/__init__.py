"""Hand-written backtestable strategies, in NautilusTrader's native ``Strategy``/``StrategyConfig``
shape (see https://nautilustrader.io — this platform's backtest engine is built on a fork,
https://github.com/MHughesDev/market_simulator, but the upstream API is what's documented here).

Each strategy module pairs a frozen ``StrategyConfig`` (its tunable parameters, serializable for
the UI) with a ``Strategy`` subclass (its event-driven runtime behavior). New strategies register
themselves in :mod:`strategies.registry` so the control plane / Asset-page backtest UI can list
them in a dropdown without importing every module eagerly.
"""

from __future__ import annotations

from strategies.registry import StrategyDescriptor, get_strategy, list_strategies

__all__ = ["StrategyDescriptor", "get_strategy", "list_strategies"]

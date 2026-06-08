"""In-process state: latest bars, regime, risk mode (Redis-backed sync can be added)."""

from __future__ import annotations

import threading
from datetime import UTC, datetime
from typing import Any

from app.contracts.events import BarEvent
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState, SystemMode


class StateManager:
    """Thread-safe snapshot of live trading state."""

    def __init__(self) -> None:
        self._lock = threading.RLock()
        self._bars: dict[str, BarEvent] = {}
        self._regime: dict[str, RegimeOutput] = {}
        self._risk = RiskState()
        self._params: dict[str, Any] = {}
        self._last_tick: datetime | None = None

    def set_bar(self, bar: BarEvent) -> None:
        with self._lock:
            self._bars[bar.symbol] = bar
            self._last_tick = datetime.now(UTC)

    def get_bar(self, symbol: str) -> BarEvent | None:
        with self._lock:
            return self._bars.get(symbol)

    def set_regime(self, symbol: str, regime: RegimeOutput) -> None:
        with self._lock:
            self._regime[symbol] = regime

    def get_regime(self, symbol: str) -> RegimeOutput | None:
        with self._lock:
            return self._regime.get(symbol)

    def set_risk_state(self, state: RiskState) -> None:
        with self._lock:
            self._risk = state

    def get_risk_state(self) -> RiskState:
        with self._lock:
            return self._risk.model_copy()

    def set_mode(self, mode: SystemMode) -> None:
        with self._lock:
            self._risk = self._risk.model_copy(update={"mode": mode})

    def set_params(self, params: dict[str, Any]) -> None:
        with self._lock:
            self._params = dict(params)

    def get_params(self) -> dict[str, Any]:
        with self._lock:
            return dict(self._params)

    def last_tick(self) -> datetime | None:
        with self._lock:
            return self._last_tick

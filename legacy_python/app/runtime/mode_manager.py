"""System mode transitions (single authority for mode changes)."""

from __future__ import annotations

from app.contracts.risk import SystemMode
from app.runtime.state_manager import StateManager


class ModeManager:
    def __init__(self, state: StateManager) -> None:
        self._state = state

    def set_mode(self, mode: SystemMode) -> SystemMode:
        self._state.set_mode(mode)
        return mode

    def get_mode(self) -> SystemMode:
        return self._state.get_risk_state().mode

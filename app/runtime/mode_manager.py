from __future__ import annotations

from threading import RLock

from app.contracts.common import SystemMode
from app.runtime.state_manager import StateManager


class ModeManager:
    """Controls runtime system mode transitions."""

    def __init__(self, state_manager: StateManager) -> None:
        self._state_manager = state_manager
        self._lock = RLock()

    def get_mode(self) -> SystemMode:
        return self._state_manager.get_state().system_mode

    def set_mode(self, mode: SystemMode) -> None:
        with self._lock:
            self._state_manager.update_mode(mode)

    def pause_new_entries(self) -> None:
        self.set_mode(SystemMode.PAUSE_NEW_ENTRIES)

    def reduce_only(self) -> None:
        self.set_mode(SystemMode.REDUCE_ONLY)

    def flatten_all(self) -> None:
        self.set_mode(SystemMode.FLATTEN_ALL)

    def maintenance(self) -> None:
        self.set_mode(SystemMode.MAINTENANCE)

    def resume_running(self) -> None:
        self.set_mode(SystemMode.RUNNING)

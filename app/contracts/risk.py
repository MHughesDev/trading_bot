from __future__ import annotations

from enum import StrEnum

from pydantic import BaseModel


class SystemMode(StrEnum):
    RUNNING = "RUNNING"
    PAUSE_NEW_ENTRIES = "PAUSE_NEW_ENTRIES"
    REDUCE_ONLY = "REDUCE_ONLY"
    FLATTEN_ALL = "FLATTEN_ALL"
    MAINTENANCE = "MAINTENANCE"


class RiskState(BaseModel):
    mode: SystemMode = SystemMode.RUNNING
    current_drawdown_pct: float = 0.0
    spread_bps: float | None = None
    data_age_seconds: float | None = None

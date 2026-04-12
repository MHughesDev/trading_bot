"""Per-asset lifecycle state (FB-AP-005) — operator/UI control for Initialize / Start / Stop."""

from __future__ import annotations

from datetime import UTC, datetime
from enum import Enum

from pydantic import BaseModel, Field


class AssetLifecycleState(str, Enum):
    """States for one symbol's trading/watch lifecycle."""

    uninitialized = "uninitialized"
    initialized_not_active = "initialized_not_active"
    active = "active"


class AssetLifecycleRecord(BaseModel):
    """Persisted lifecycle row for a single symbol."""

    schema_version: str = Field(default="1", description="Record format version")
    symbol: str = Field(..., min_length=1, description="Canonical symbol (matches manifest path)")
    state: AssetLifecycleState
    updated_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        description="ISO-8601 UTC when this record was last written",
    )

    @staticmethod
    def bump_timestamp() -> str:
        return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")

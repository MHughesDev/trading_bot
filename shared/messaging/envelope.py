"""Event envelope contracts for service-to-service messaging."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any
from uuid import UUID, uuid4

from pydantic import BaseModel, Field


class EventEnvelope(BaseModel):
    """Standard event envelope used across internal async topics."""

    event_id: UUID = Field(default_factory=uuid4)
    event_type: str
    event_version: str = "v1"
    trace_id: str
    correlation_id: str | None = None
    producer_service: str
    symbol: str | None = None
    partition_key: str | None = None
    ts_event: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    ts_ingest: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    payload: dict[str, Any]
    schema_hash: str | None = None

"""Canonical common field conventions (FB-CAN-062, APEX Feature Schema §4).

- **Timestamps:** UTC only; naive datetimes are interpreted as UTC (explicit, never local).
- **Confidence / freshness:** normalized to ``[0.0, 1.0]``; NaN/inf and non-finite inputs clip or fall back deterministically.
- **Missing / malformed scalars:** callers should degrade (clip/fallback), not propagate undefined floats.
"""

from __future__ import annotations

import math
from datetime import UTC, datetime
from typing import Any


def ensure_utc_datetime(ts: datetime) -> datetime:
    """Return ``ts`` in UTC. Naive datetimes are treated as UTC."""
    if ts.tzinfo is None:
        return ts.replace(tzinfo=UTC)
    return ts.astimezone(UTC)


def clip_unit_interval(x: Any, *, default: float = 0.0) -> float:
    """Clip to ``[0.0, 1.0]``. Non-finite or non-numeric → ``default``."""
    try:
        v = float(x)
    except (TypeError, ValueError):
        return default
    if math.isnan(v) or math.isinf(v):
        return default
    return max(0.0, min(1.0, v))


def validate_confidence_like(x: Any, *, field: str = "confidence") -> float:
    """Normalized confidence in ``[0, 1]`` (§4.3)."""
    return clip_unit_interval(x, default=0.0)


def validate_freshness_like(x: Any, *, field: str = "freshness") -> float:
    """Normalized freshness in ``[0, 1]`` (§4.4)."""
    return clip_unit_interval(x, default=0.0)


def clip_symmetric_unit(x: Any, *, default: float = 0.0) -> float:
    """Clip to ``[-1.0, 1.0]`` (e.g. signed scores). Non-finite → ``default``."""
    try:
        v = float(x)
    except (TypeError, ValueError):
        return default
    if math.isnan(v) or math.isinf(v):
        return default
    return max(-1.0, min(1.0, v))


def validate_decision_boundary_input_timestamps(inp: Any) -> None:
    """Assert all snapshot timestamps are timezone-aware (§4.2 — no naive local exchange times)."""
    from app.contracts.decision_snapshots import DecisionBoundaryInput

    if not isinstance(inp, DecisionBoundaryInput):
        raise TypeError("expected DecisionBoundaryInput")
    for part in (
        inp.market,
        inp.structural,
        inp.safety,
        inp.execution_feedback,
        inp.service_config,
    ):
        ts = part.timestamp
        if ts.tzinfo is None:
            raise ValueError(f"{type(part).__name__}.timestamp must be timezone-aware (use UTC)")

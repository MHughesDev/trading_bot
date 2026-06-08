"""Trace helpers for event envelopes."""

from __future__ import annotations

from uuid import uuid4


def new_trace_id() -> str:
    """Create a new trace id suitable for envelope propagation."""
    return str(uuid4())

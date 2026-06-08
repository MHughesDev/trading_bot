"""Per-asset lifecycle state (FB-AP-005) — operator-facing state machine for init / watch."""

from __future__ import annotations

from enum import StrEnum


class AssetLifecycleState(StrEnum):
    """States for a single tradable symbol's model + watch lifecycle."""

    uninitialized = "uninitialized"
    initialized_not_active = "initialized_not_active"
    active = "active"

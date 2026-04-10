"""Known-future cyclical features per horizon (human spec §7.5)."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import numpy as np


def known_future_features(
    anchor_time: datetime,
    horizon_steps: int,
    *,
    base_interval_seconds: int = 60,
) -> np.ndarray:
    """
    Returns X_known [H, F_known]: sin/cos minute-of-hour (2), hour-of-day (2), day-of-week (2) = 6 features.

    Times are anchor_time + h * base_interval for h in 1..H.
    """
    H = horizon_steps
    F = 6
    out = np.zeros((H, F), dtype=np.float64)
    if anchor_time.tzinfo is None:
        anchor_time = anchor_time.replace(tzinfo=UTC)
    for h in range(H):
        t = anchor_time + timedelta(seconds=base_interval_seconds * (h + 1))
        minute = t.minute + t.second / 60.0
        hour = t.hour + minute / 60.0
        dow = t.weekday()
        out[h, 0] = np.sin(2 * np.pi * minute / 60.0)
        out[h, 1] = np.cos(2 * np.pi * minute / 60.0)
        out[h, 2] = np.sin(2 * np.pi * hour / 24.0)
        out[h, 3] = np.cos(2 * np.pi * hour / 24.0)
        out[h, 4] = np.sin(2 * np.pi * dow / 7.0)
        out[h, 5] = np.cos(2 * np.pi * dow / 7.0)
    return out

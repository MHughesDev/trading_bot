"""Sliding-window conformal calibration (human spec §16)."""

from __future__ import annotations

from collections import deque


class SlidingConformalCalibrator:
    def __init__(self, alpha: float, window_size: int) -> None:
        self.alpha = float(alpha)
        self.window_size = int(window_size)
        self._scores: deque[float] = deque(maxlen=window_size)

    def update(self, y_true: float, q_low: float, q_high: float) -> None:
        eps = max(q_low - y_true, y_true - q_high, 0.0)
        self._scores.append(eps)

    def calibrate(self, q_low: float, q_high: float) -> tuple[float, float]:
        if not self._scores:
            return q_low, q_high
        sorted_s = sorted(self._scores)
        idx = min(int((1.0 - self.alpha) * (len(sorted_s) - 1)), len(sorted_s) - 1)
        q_eps = sorted_s[max(0, idx)]
        return q_low - q_eps, q_high + q_eps

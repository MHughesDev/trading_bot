"""Sliding-window conformal calibration (human spec §16) with JSON persistence (FB-FR-PG3)."""

from __future__ import annotations

import json
from collections import deque
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from app.contracts.forecast_packet import ForecastPacket


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

    def to_dict(self) -> dict[str, Any]:
        return {
            "alpha": self.alpha,
            "window_size": self.window_size,
            "scores": list(self._scores),
        }

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> SlidingConformalCalibrator:
        alpha = float(d["alpha"])
        window_size = int(d["window_size"])
        obj = cls(alpha, window_size)
        scores = d.get("scores") or []
        for s in scores:
            obj._scores.append(float(s))
        return obj


class MultiHorizonConformal:
    """One sliding calibrator per forecast horizon; persistable as JSON."""

    def __init__(self, calibrators: Sequence[SlidingConformalCalibrator]) -> None:
        self._calibrators = list(calibrators)

    @classmethod
    def create(cls, num_horizons: int, *, alpha: float, window_size: int) -> MultiHorizonConformal:
        return cls(
            [SlidingConformalCalibrator(alpha, window_size) for _ in range(num_horizons)],
        )

    def __len__(self) -> int:
        return len(self._calibrators)

    def update_horizon(self, horizon_index: int, y_true: float, q_low: float, q_high: float) -> None:
        self._calibrators[horizon_index].update(y_true, q_low, q_high)

    def apply_to_quantiles(
        self,
        q_low: list[float],
        q_med: list[float],
        q_high: list[float],
    ) -> tuple[list[float], list[float], list[float]]:
        """Return calibrated low/med/high lists (median unchanged; interval may widen)."""
        n = min(len(self._calibrators), len(q_low), len(q_med), len(q_high))
        out_lo = list(q_low)
        out_hi = list(q_high)
        out_md = list(q_med)
        for h in range(n):
            lo, hi = self._calibrators[h].calibrate(out_lo[h], out_hi[h])
            out_lo[h], out_hi[h] = lo, hi
        return out_lo, out_md, out_hi

    def apply_to_packet(self, pkt: ForecastPacket) -> ForecastPacket:
        lo, md, hi = self.apply_to_quantiles(pkt.q_low, pkt.q_med, pkt.q_high)
        iv = [hi[i] - lo[i] for i in range(len(lo))]
        diag = dict(pkt.forecast_diagnostics)
        diag["conformal_applied"] = True
        return ForecastPacket(
            timestamp=pkt.timestamp,
            horizons=list(pkt.horizons),
            q_low=lo,
            q_med=md,
            q_high=hi,
            interval_width=iv,
            regime_vector=list(pkt.regime_vector),
            confidence_score=pkt.confidence_score,
            ensemble_variance=list(pkt.ensemble_variance),
            ood_score=pkt.ood_score,
            forecast_diagnostics=diag,
            packet_schema_version=pkt.packet_schema_version,
            source_checkpoint_id=pkt.source_checkpoint_id,
        )

    def to_dict(self) -> dict[str, Any]:
        return {"version": 1, "calibrators": [c.to_dict() for c in self._calibrators]}

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> MultiHorizonConformal:
        cals = [SlidingConformalCalibrator.from_dict(x) for x in d["calibrators"]]
        return cls(cals)


def save_conformal_state(path: str | Path, bundle: MultiHorizonConformal) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(bundle.to_dict(), indent=2), encoding="utf-8")


def load_conformal_state(path: str | Path) -> MultiHorizonConformal:
    p = Path(path)
    raw = json.loads(p.read_text(encoding="utf-8"))
    return MultiHorizonConformal.from_dict(raw)

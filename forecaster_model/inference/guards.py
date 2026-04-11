"""Forecast rejection guards (human spec §24)."""

from __future__ import annotations

from dataclasses import dataclass

from app.contracts.forecast_packet import ForecastPacket


@dataclass
class ForecastGuardConfig:
    max_interval_width: float = 1.0
    max_ensemble_variance: float = 1.0
    max_ood_score: float = 0.95
    min_confidence: float = 1e-6
    # FB-FR-PG8: z-score vs reference mean for mean(|q_med|)
    drift_reference_abs_qmed_mean: float | None = None
    drift_max_zscore: float = 6.0
    drift_reference_std: float = 1e-3


class ForecastGuard:
    def __init__(self, cfg: ForecastGuardConfig | None = None) -> None:
        self._cfg = cfg or ForecastGuardConfig()

    def check(self, pkt: ForecastPacket) -> tuple[bool, list[str]]:
        reasons: list[str] = []
        if max(pkt.interval_width) > self._cfg.max_interval_width:
            reasons.append("interval_width")
        if max(pkt.ensemble_variance) > self._cfg.max_ensemble_variance:
            reasons.append("ensemble_disagreement")
        if pkt.ood_score > self._cfg.max_ood_score:
            reasons.append("ood")
        conf = (
            float(pkt.confidence_score)
            if isinstance(pkt.confidence_score, (int, float))
            else max(pkt.confidence_score)
        )
        if conf < self._cfg.min_confidence:
            reasons.append("low_confidence")
        if self._cfg.drift_reference_abs_qmed_mean is not None:
            m = float(sum(abs(x) for x in pkt.q_med) / max(len(pkt.q_med), 1))
            ref = self._cfg.drift_reference_abs_qmed_mean
            sigma = max(self._cfg.drift_reference_std, 1e-9)
            z = abs(m - ref) / sigma
            if z > self._cfg.drift_max_zscore:
                reasons.append("feature_drift")
        return len(reasons) == 0, reasons

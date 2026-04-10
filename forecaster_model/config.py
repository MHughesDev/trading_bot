"""YAML-friendly forecaster configuration (subset of human spec §6)."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class ForecasterConfig:
    base_interval_seconds: int = 60
    history_length: int = 64
    forecast_horizon: int = 4
    quantiles: tuple[float, ...] = (0.1, 0.5, 0.9)
    feature_windows: tuple[int, ...] = (4, 16, 64)
    num_regime_dims: int = 4
    calibration_enabled: bool = False
    ensemble_members: int = 1
    extra: dict[str, object] = field(default_factory=dict)

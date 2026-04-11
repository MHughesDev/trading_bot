"""Forecaster configuration (human forecaster spec §6)."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class ForecasterConfig:
    """Defaults align with `docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD` §7.1."""

    base_interval_seconds: int = 60
    history_length: int = 128
    forecast_horizon: int = 8
    quantiles: tuple[float, ...] = (0.1, 0.5, 0.9)
    feature_windows: tuple[int, ...] = (4, 16, 64)
    num_regime_dims: int = 4
    latent_width: int = 32
    recurrent_hidden_width: int = 128
    branch_scales: tuple[int, ...] = (1, 5, 20, 100)
    calibration_enabled: bool = False
    conformal_alpha: float = 0.1
    conformal_window_size: int = 100
    ensemble_members: int = 1
    extra: dict[str, object] = field(default_factory=dict)


__all__ = ["ForecasterConfig"]

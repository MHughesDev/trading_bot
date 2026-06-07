"""FB-AUDIT-01: training ForecasterConfig matches runtime defaults."""

from __future__ import annotations

from training_pipeline.orchestration.training_campaign import _cfg_from_spec
from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from app.config.settings import AppSettings


def test_cfg_from_spec_matches_runtime_defaults() -> None:
    base = ForecasterConfig()
    for mode in ("initial", "nightly"):
        cfg = _cfg_from_spec(mode, settings=AppSettings())
        assert cfg.history_length == base.history_length
        assert cfg.forecast_horizon == base.forecast_horizon
        assert cfg.quantiles == base.quantiles

"""FB-SPEC-02: real-data QuantileForecasterArtifact is loaded and served by DecisionPipeline.

Wires the only real-data-trained forecaster (sklearn quantile, joblib) into the live/replay
serving path. Verifies the artifact takes precedence over the numpy/torch forward and that its
geometry (feature_windows / history_length / regime dims) is read back from the artifact, so a
train/serve config drift cannot silently break `feature_dim` alignment.
"""

from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

import numpy as np
import polars as pl

from app.config.settings import load_settings
from app.contracts.risk import RiskState
from legacy.decision_pipeline.decision_engine.pipeline import DecisionPipeline, _load_quantile_forecaster_if_configured
from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from training_pipeline.forecaster_training.real_data_fit import fit_quantile_forecaster_from_bars


def _random_walk_bars(n: int = 400, seed: int = 7) -> pl.DataFrame:
    rng = np.random.default_rng(seed)
    close = 100.0 + rng.normal(0.0, 0.5, size=n).cumsum()
    high = close + np.abs(rng.normal(0.0, 0.2, size=n))
    low = close - np.abs(rng.normal(0.0, 0.2, size=n))
    open_ = close + rng.normal(0.0, 0.1, size=n)
    volume = np.abs(rng.normal(1000.0, 50.0, size=n))
    return pl.DataFrame(
        {"open": open_, "high": high, "low": low, "close": close, "volume": volume}
    )


def _small_cfg() -> ForecasterConfig:
    # Smaller geometry than runtime defaults — exercises that serving reads geometry from the
    # artifact, not from ambient ForecasterConfig() defaults.
    return ForecasterConfig(
        history_length=32,
        forecast_horizon=2,
        quantiles=(0.1, 0.5, 0.9),
        feature_windows=(4, 8),
        num_regime_dims=4,
    )


def test_quantile_artifact_loaded_and_served(tmp_path: Path) -> None:
    cfg = _small_cfg()
    bars = _random_walk_bars()
    artifact = fit_quantile_forecaster_from_bars(
        bars, cfg, train_range=range(0, bars.height), data_snapshot_id="unit-test"
    )
    art_path = tmp_path / "forecaster_quantile_real.joblib"
    artifact.save(art_path)

    settings = load_settings().model_copy(
        update={
            "market_data_symbols": ["BTC-USD"],
            "models_forecaster_quantile_path": str(art_path),
        }
    )

    # Loader resolves the configured artifact.
    assert _load_quantile_forecaster_if_configured(settings) is not None

    pipe = DecisionPipeline(settings=settings)
    pipe.step(
        "BTC-USD",
        {"close": 100.0, "volume": 1000.0},
        2.0,
        RiskState(),
        mid_price=100.0,
        portfolio_equity_usd=100_000.0,
        data_timestamp=datetime(2026, 1, 1, tzinfo=UTC),
    )

    pkt = pipe.last_forecast_packet
    assert pkt is not None
    assert pkt.forecast_diagnostics.get("methodology") == "quantile_ohlc_v1"
    assert pkt.forecast_diagnostics.get("data_snapshot_id") == "unit-test"


def test_no_quantile_path_uses_numpy_reference(tmp_path: Path) -> None:
    settings = load_settings().model_copy(
        update={"market_data_symbols": ["BTC-USD"], "models_forecaster_quantile_path": None}
    )
    pipe = DecisionPipeline(settings=settings)
    pipe.step(
        "BTC-USD",
        {"close": 100.0, "volume": 1000.0},
        2.0,
        RiskState(),
        mid_price=100.0,
        portfolio_equity_usd=100_000.0,
        data_timestamp=datetime(2026, 1, 1, tzinfo=UTC),
    )
    pkt = pipe.last_forecast_packet
    assert pkt is not None
    # Falls back to the numpy-reference forward (not the quantile methodology).
    assert pkt.forecast_diagnostics.get("methodology") != "quantile_ohlc_v1"

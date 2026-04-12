"""FB-AP-037: nightly RL skipped when no trade activity in lookback."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

import polars as pl
import pytest

from app.config.settings import AppSettings
from forecaster_model.training.real_data_fit import QuantileForecasterArtifact
from orchestration.rl_real_data_eval import RLEpisodeMetrics
from orchestration.training_campaign import run_training_campaign


def _bars(n: int = 120) -> pl.DataFrame:
    t0 = datetime(2026, 1, 1, tzinfo=UTC)
    return pl.DataFrame(
        {
            "timestamp": [t0 + timedelta(minutes=i) for i in range(n)],
            "open": [1.0 + i * 1e-6 for i in range(n)],
            "high": [1.01 + i * 1e-6 for i in range(n)],
            "low": [0.99 + i * 1e-6 for i in range(n)],
            "close": [1.0 + i * 1e-6 for i in range(n)],
            "volume": [1.0] * n,
        }
    )


def _dummy_artifact() -> QuantileForecasterArtifact:
    return QuantileForecasterArtifact(
        feature_dim=4,
        horizons=[1],
        quantiles=(0.1, 0.5, 0.9),
        models={},
        config_snapshot={},
        data_snapshot_id="test",
    )


@pytest.mark.parametrize("had_trade", [False, True])
def test_nightly_rl_skipped_without_trade_markers(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, had_trade: bool
) -> None:
    monkeypatch.setattr("orchestration.training_campaign.is_on", lambda: True)
    monkeypatch.setattr(
        "orchestration.training_campaign.fetch_symbol_bars_sync",
        lambda *_a, **_k: _bars(),
    )
    monkeypatch.setattr(
        "orchestration.training_campaign.fit_quantile_forecaster_from_bars",
        lambda *_a, **_k: _dummy_artifact(),
    )
    monkeypatch.setattr(
        "orchestration.training_campaign._eval_pinball_on_range",
        lambda *_a, **_k: {"pinball_mean": 0.1, "n": 5.0},
    )
    rl_calls: list[int] = []

    def _track_rl(*_a, **_k):
        rl_calls.append(1)
        return RLEpisodeMetrics(
            total_return=0.0,
            sharpe_like=0.0,
            max_drawdown=0.0,
            turnover=0.0,
            trade_count=0,
            steps=1,
        )

    monkeypatch.setattr(
        "orchestration.training_campaign.run_heuristic_rollout_on_range",
        _track_rl,
    )
    monkeypatch.setattr(
        "orchestration.training_campaign.symbol_had_trade_in_lookback",
        lambda *_a, **_k: had_trade,
    )
    class _Prom:
        def to_dict(self) -> dict:
            return {}

    monkeypatch.setattr(
        "orchestration.training_campaign.decide_forecaster_promotion_stub",
        lambda **k: _Prom(),
    )
    monkeypatch.setattr(
        "orchestration.training_campaign.write_promotion_sidecar",
        lambda *a, **k: tmp_path / "promo.json",
    )

    s = AppSettings(scheduler_nightly_rl_requires_trade=True)
    r = run_training_campaign(
        mode="nightly",
        symbol="BTC-USD",
        artifact_dir=tmp_path,
        settings=s,
        lookback_days=30,
    )
    if had_trade:
        assert r.get("rl_skipped") is None
        assert len(rl_calls) >= 1
    else:
        assert r.get("rl_skipped", {}).get("reason") == "no_trade_markers_in_lookback"
        assert r["rl_runs"] == []
        assert rl_calls == []

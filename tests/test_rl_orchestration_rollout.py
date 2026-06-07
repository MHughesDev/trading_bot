"""Real policy rollouts in orchestration: train actor-critic on real bars, eval, persist policy."""

from __future__ import annotations

import numpy as np
import polars as pl

from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from training_pipeline.forecaster_training.real_data_fit import fit_quantile_forecaster_from_bars
from training_pipeline.orchestration.rl_real_data_eval import (
    RLEpisodeMetrics,
    run_heuristic_rollout_on_range,
    run_policy_rollout_on_range,
    train_actor_critic_on_range,
)
from legacy.decision_pipeline.policy_model.policy.policy_network import PolicyNetwork

_CFG = ForecasterConfig(
    history_length=8,
    forecast_horizon=2,
    feature_windows=(2, 4),
    quantiles=(0.1, 0.5, 0.9),
    num_regime_dims=4,
)


def _bars(n: int = 140) -> pl.DataFrame:
    t = np.arange(n)
    close = 100.0 + 3.0 * np.sin(t / 7.0) + 0.5 * np.cos(t / 3.0)
    open_ = np.concatenate([[close[0]], close[:-1]])
    high = np.maximum(open_, close) + 0.1
    low = np.minimum(open_, close) - 0.1
    vol = np.full(n, 1_000_000.0)
    return pl.DataFrame(
        {"open": open_, "high": high, "low": low, "close": close, "volume": vol}
    )


def _artifact(bars: pl.DataFrame):
    return fit_quantile_forecaster_from_bars(bars, _CFG, train_range=range(0, 120))


def test_train_actor_critic_returns_metrics_and_persists_policy(tmp_path) -> None:
    bars = _bars()
    artifact = _artifact(bars)
    pol_path = tmp_path / "policy" / "policy_mlp.npz"
    metrics = train_actor_critic_on_range(
        bars,
        range(0, 80),
        artifact,
        _CFG,
        max_steps=24,
        epochs=2,
        seed=1,
        save_policy_path=pol_path,
    )
    assert isinstance(metrics, RLEpisodeMetrics)
    assert metrics.steps > 0
    assert pol_path.exists()

    # The persisted policy is loadable and can drive a deterministic eval rollout.
    loaded = PolicyNetwork()
    loaded.load(str(pol_path))
    eval_metrics = run_policy_rollout_on_range(
        bars, range(0, 80), artifact, _CFG, policy=loaded, max_steps=24
    )
    assert isinstance(eval_metrics, RLEpisodeMetrics)
    assert eval_metrics.steps > 0


def test_heuristic_rollout_still_works(tmp_path) -> None:
    bars = _bars()
    artifact = _artifact(bars)
    metrics = run_heuristic_rollout_on_range(bars, range(0, 80), artifact, _CFG, max_steps=24)
    assert isinstance(metrics, RLEpisodeMetrics)
    assert metrics.steps > 0
    # Fee accounting is now non-degenerate: turnover implies at least one counted trade.
    if metrics.turnover > 1e-3:
        assert metrics.trade_count >= 1

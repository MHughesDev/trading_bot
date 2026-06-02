"""Real PyTorch forecaster trainer: pinball loss decreases, checkpoint is serving-loadable."""

from __future__ import annotations

import numpy as np
import pytest

torch = pytest.importorskip("torch")  # skips when the optional [models_torch] extra is absent

import polars as pl  # noqa: E402

from forecaster_model.config import ForecasterConfig  # noqa: E402
from forecaster_model.inference.torch_infer import (  # noqa: E402
    forward_torch_quantiles,
    load_torch_forecaster_checkpoint,
)
from forecaster_model.training.torch_trainer import (  # noqa: E402
    build_torch_training_samples,
    train_forecaster_torch,
)

_CFG = ForecasterConfig(
    history_length=16,
    forecast_horizon=3,
    feature_windows=(2, 4),
    num_regime_dims=4,
    quantiles=(0.1, 0.5, 0.9),
)


def _sine_bars(n: int = 160) -> pl.DataFrame:
    t = np.arange(n)
    close = 100.0 + 5.0 * np.sin(t / 9.0) + np.cos(t / 4.0)
    open_ = np.concatenate([[close[0]], close[:-1]])
    high = np.maximum(open_, close) + 0.2
    low = np.minimum(open_, close) - 0.2
    vol = np.full(n, 1_000_000.0)
    return pl.DataFrame(
        {"open": open_, "high": high, "low": low, "close": close, "volume": vol}
    )


def test_training_samples_shapes() -> None:
    x_obs, x_known, r, y = build_torch_training_samples(_sine_bars(120), _CFG)
    f_obs = 5 + 2 * len(_CFG.feature_windows)
    assert x_obs.shape[1:] == (_CFG.history_length, f_obs)
    assert x_known.shape[1:] == (_CFG.forecast_horizon, 6)
    assert r.shape[1] == _CFG.num_regime_dims
    assert y.shape[1] == _CFG.forecast_horizon
    assert x_obs.shape[0] == x_known.shape[0] == r.shape[0] == y.shape[0] > 16


def test_train_on_real_bars_reduces_loss_and_serves(tmp_path) -> None:
    meta = train_forecaster_torch(
        artifact_dir=tmp_path,
        bars=_sine_bars(200),
        cfg=_CFG,
        epochs=25,
        batch_size=32,
        device="cpu",
        seed=1,
    )
    assert meta["trainer"] == "torch_pinball_mlp"
    assert meta["data"] == "real_bars"
    # Learning happened: final epoch pinball loss is below the first epoch's.
    assert meta["final_epoch_loss"] < meta["first_epoch_loss"]

    pt = tmp_path / "forecaster_torch.pt"
    assert pt.exists()

    # Serving path can load the checkpoint and run a forward — proves drop-in compatibility.
    model, device, loaded_cfg = load_torch_forecaster_checkpoint(pt, cfg=_CFG)
    assert loaded_cfg.forecast_horizon == _CFG.forecast_horizon
    f_obs = 5 + 2 * len(_CFG.feature_windows)
    x_obs = np.zeros((_CFG.history_length, f_obs), dtype=np.float64)
    x_known = np.zeros((_CFG.forecast_horizon, 6), dtype=np.float64)
    r_cur = np.full(_CFG.num_regime_dims, 0.25, dtype=np.float64)
    out = forward_torch_quantiles(x_obs, x_known, r_cur, model=model, device=device)
    assert out.shape == (_CFG.forecast_horizon, len(_CFG.quantiles))
    assert np.all(np.isfinite(out))


def test_train_synthetic_fallback_when_no_bars(tmp_path) -> None:
    meta = train_forecaster_torch(
        artifact_dir=tmp_path, cfg=_CFG, epochs=3, device="cpu", seed=7
    )
    assert meta["data"] == "synthetic_random_walk"
    assert meta["samples"] >= 16
    assert (tmp_path / "forecaster_torch.pt").exists()

"""FB-SPEC-02: NPZ forecaster weights + optional policy MLP on decision path."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from app.config.settings import AppSettings
from decision_engine.pipeline import DecisionPipeline
from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_weights import capture_forecaster_weights_from_seed, save_forecaster_weights
from forecaster_model.models.numpy_reference import forward_numpy_reference
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


def test_forecaster_npz_matches_rng_forward() -> None:
    cfg = ForecasterConfig(history_length=32, forecast_horizon=4, branch_scales=(1, 5))
    seed = 7
    rng = np.random.default_rng(0)
    L, F = 32, 11
    x_obs = rng.normal(size=(L, F))
    x_known = rng.normal(size=(cfg.forecast_horizon, 6))
    r_cur = np.ones(cfg.num_regime_dims) / cfg.num_regime_dims
    y1, _ = forward_numpy_reference(x_obs, x_known, r_cur, cfg, seed=seed)
    bundle = capture_forecaster_weights_from_seed(cfg, seed=seed, f_obs=F)
    y2, _ = forward_numpy_reference(x_obs, x_known, r_cur, cfg, seed=seed, weight_bundle=bundle)
    np.testing.assert_allclose(y1, y2, rtol=1e-9, atol=1e-9)


def test_pipeline_loads_npz_weights(tmp_path: Path) -> None:
    cfg = ForecasterConfig()
    bundle = capture_forecaster_weights_from_seed(cfg, seed=42)
    wpath = tmp_path / "f.npz"
    save_forecaster_weights(wpath, bundle)

    policy = MultiBranchMLPPolicy(seed=99)
    ppath = tmp_path / "p.npz"
    policy.save(ppath)

    settings = AppSettings(
        models_forecaster_weights_path=str(wpath),
        models_policy_mlp_path=str(ppath),
        models_forecaster_checkpoint_id="test-chk",
    )
    dp = DecisionPipeline(settings=settings)
    comps = dp._get_or_create_serving_components(settings)
    assert comps.forecaster_weight_bundle is not None
    assert comps.policy_system is not None


def test_preflight_fixture_unchanged() -> None:
    fixture = Path(__file__).resolve().parent / "fixtures" / "forecaster_golden_packet.json"
    raw = json.loads(fixture.read_text(encoding="utf-8"))
    assert raw["schema_version"] >= 2
    assert len(raw["q_med"]) == raw["forecast_horizon"]

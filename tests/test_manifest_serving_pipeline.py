"""FB-AP-003 / FB-AP-004: per-asset manifest guards on DecisionPipeline."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.risk import RiskState
from app.runtime import asset_model_registry as reg_mod
from decision_engine.pipeline import DecisionPipeline
from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_weights import capture_forecaster_weights_from_seed, save_forecaster_weights
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


def _features(close: float = 50_000.0) -> dict[str, float]:
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = close
    feats["volume"] = 1e6
    return feats


def test_manifest_without_forecaster_paths_abstains(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Registry manifest exists but no torch/NPZ — must not fall back to global env weights."""
    monkeypatch.setattr(reg_mod, "_DEFAULT_DIR", tmp_path)
    manifest = AssetModelManifest(
        canonical_symbol="BTC-USD",
        forecaster_weights_path=None,
        forecaster_torch_path=None,
    )
    reg_mod.save_manifest(manifest)

    global_npz = tmp_path / "global_f.npz"
    cfg = ForecasterConfig()
    save_forecaster_weights(global_npz, capture_forecaster_weights_from_seed(cfg, seed=1))
    settings = AppSettings(models_forecaster_weights_path=str(global_npz))

    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    _, fc, route, proposal = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is None
    assert route.route_id.value == "NO_TRADE"
    assert proposal is None
    assert fc.uncertainty == 1.0


def test_manifest_resolves_forecaster_and_steps(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(reg_mod, "_DEFAULT_DIR", tmp_path)
    cfg = ForecasterConfig()
    wpath = tmp_path / "per_asset_f.npz"
    save_forecaster_weights(wpath, capture_forecaster_weights_from_seed(cfg, seed=42))
    manifest = AssetModelManifest(
        canonical_symbol="BTC-USD",
        forecaster_weights_path=str(wpath),
    )
    reg_mod.save_manifest(manifest)

    settings = AppSettings()
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    _, _, _, _ = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("pipeline") == "master_spec"


def test_manifest_policy_path_missing_abstains(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(reg_mod, "_DEFAULT_DIR", tmp_path)
    cfg = ForecasterConfig()
    wpath = tmp_path / "f.npz"
    save_forecaster_weights(wpath, capture_forecaster_weights_from_seed(cfg, seed=3))
    manifest = AssetModelManifest(
        canonical_symbol="ETH-USD",
        forecaster_weights_path=str(wpath),
        policy_mlp_path=str(tmp_path / "nope.npz"),
    )
    reg_mod.save_manifest(manifest)

    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    _, _, route, proposal = pipe.step(
        "ETH-USD",
        _features(close=3000.0),
        spread_bps=5.0,
        risk=risk,
        mid_price=3000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is None
    assert route.route_id.value == "NO_TRADE"
    assert proposal is None


def test_manifest_policy_optional_heuristic_ok(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """No policy_mlp in manifest → heuristic policy is allowed."""
    monkeypatch.setattr(reg_mod, "_DEFAULT_DIR", tmp_path)
    cfg = ForecasterConfig()
    wpath = tmp_path / "f2.npz"
    save_forecaster_weights(wpath, capture_forecaster_weights_from_seed(cfg, seed=5))
    manifest = AssetModelManifest(
        canonical_symbol="SOL-USD",
        forecaster_weights_path=str(wpath),
        policy_mlp_path=None,
    )
    reg_mod.save_manifest(manifest)

    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    _, _, _, _ = pipe.step(
        "SOL-USD",
        _features(close=100.0),
        spread_bps=5.0,
        risk=risk,
        mid_price=100.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None


def test_manifest_with_policy_npz_steps(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(reg_mod, "_DEFAULT_DIR", tmp_path)
    cfg = ForecasterConfig()
    f_npz = tmp_path / "f3.npz"
    save_forecaster_weights(f_npz, capture_forecaster_weights_from_seed(cfg, seed=7))
    pol = MultiBranchMLPPolicy(seed=11)
    p_npz = tmp_path / "p3.npz"
    pol.save(p_npz)
    manifest = AssetModelManifest(
        canonical_symbol="BTC-USD",
        forecaster_weights_path=str(f_npz),
        policy_mlp_path=str(p_npz),
    )
    reg_mod.save_manifest(manifest)

    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    _, _, _, proposal = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    # Proposal may be None depending on policy output; key is we did not abstain
    assert proposal is None or proposal.symbol == "BTC-USD"

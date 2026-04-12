"""FB-AP-003 / FB-AP-004 / FB-AP-042: per-asset manifest guards + integration tests."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.risk import RiskState
from app.runtime import asset_model_registry as reg
from decision_engine import pipeline as pipeline_mod
from decision_engine.pipeline import DecisionPipeline
from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_weights import capture_forecaster_weights_from_seed, save_forecaster_weights
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


@pytest.fixture
def registry_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    d = tmp_path / "manifests"
    d.mkdir(parents=True)
    monkeypatch.setenv("NM_ASSET_MODEL_REGISTRY_DIR", str(d))
    monkeypatch.setattr(reg, "_DEFAULT_DIR", d)
    return d


def _write_manifest(registry_dir: Path, symbol: str, **kwargs: str) -> None:
    m = AssetModelManifest(canonical_symbol=symbol, **kwargs)
    p = registry_dir / f"{symbol}.json"
    p.write_text(m.model_dump_json(indent=2), encoding="utf-8")


def test_manifest_mode_abstains_without_manifest(registry_dir: Path) -> None:
    pipeline_mod._serving_mode_logged = False
    settings = AppSettings(models_use_asset_manifest_paths=True)
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    out = pipe.step("ETH-USD", feats, 5.0, risk, mid_price=50_000.0, portfolio_equity_usd=100_000.0)
    regime, _fc, route, proposal = out
    assert route.route_id.value == "NO_TRADE"
    assert proposal is None
    assert pipe.last_forecast_packet is None
    assert regime.confidence == 0.0


def test_manifest_mode_uses_per_symbol_npz_and_policy(registry_dir: Path, tmp_path: Path) -> None:
    cfg = ForecasterConfig()
    bundle = capture_forecaster_weights_from_seed(cfg, seed=42)
    wpath = tmp_path / "btc_f.npz"
    save_forecaster_weights(wpath, bundle)

    policy = MultiBranchMLPPolicy(seed=99)
    ppath = tmp_path / "btc_p.npz"
    policy.save(ppath)

    _write_manifest(
        registry_dir,
        "BTC-USD",
        forecaster_weights_path=str(wpath),
        policy_mlp_path=str(ppath),
    )

    pipeline_mod._serving_mode_logged = False
    settings = AppSettings(
        models_use_asset_manifest_paths=True,
        models_forecaster_weights_path=str(tmp_path / "wrong.npz"),
        models_policy_mlp_path=str(tmp_path / "wrong_policy.npz"),
    )
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    pipe.step(
        "BTC-USD",
        feats,
        5.0,
        risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    pkt = pipe.last_forecast_packet
    assert pkt is not None
    assert pkt.forecast_diagnostics.get("symbol") == "BTC-USD"


def test_manifest_round_trip_load_matches_saved(registry_dir: Path, tmp_path: Path) -> None:
    cfg = ForecasterConfig()
    bundle = capture_forecaster_weights_from_seed(cfg, seed=7)
    wpath = tmp_path / "sym.npz"
    save_forecaster_weights(wpath, bundle)

    policy = MultiBranchMLPPolicy(seed=3)
    ppath = tmp_path / "sym_p.npz"
    policy.save(ppath)

    m = AssetModelManifest(
        canonical_symbol="SOL-USD",
        forecaster_weights_path=str(wpath),
        policy_mlp_path=str(ppath),
    )
    reg.save_manifest(m)
    loaded = reg.load_manifest("SOL-USD")
    assert loaded is not None
    assert loaded.canonical_symbol == "SOL-USD"
    assert loaded.forecaster_weights_path == str(wpath)


def test_corrupt_manifest_file_abstains(registry_dir: Path) -> None:
    bad = registry_dir / "ETH-USD.json"
    bad.write_text('{"canonical_symbol": "BTC-USD"}', encoding="utf-8")

    pipeline_mod._serving_mode_logged = False
    settings = AppSettings(models_use_asset_manifest_paths=True)
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 1.0
    feats["volume"] = 1.0
    out = pipe.step("ETH-USD", feats, 5.0, risk, mid_price=1.0, portfolio_equity_usd=100_000.0)
    _r, _fc, route, proposal = out
    assert route.route_id.value == "NO_TRADE"
    assert proposal is None
    assert pipe.last_forecast_packet is None

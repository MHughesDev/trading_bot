"""Per-asset model manifest schema, registry I/O, and DecisionPipeline binding (FB-AP-001/002/003)."""

from __future__ import annotations

import json
import logging
from pathlib import Path

import pytest

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifestV1, ForecasterArtifactPaths
from app.contracts.risk import RiskState
from data_plane.storage import asset_model_registry as reg
from decision_engine import pipeline as pipeline_mod
from decision_engine.pipeline import DecisionPipeline


def _features(close: float = 50_000.0) -> dict[str, float]:
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = close
    feats["volume"] = 1e6
    return feats


def test_manifest_validates_and_rejects_symbol_mismatch() -> None:
    m = AssetModelManifestV1(
        manifest_id="mid-001",
        symbol="BTC-USD",
        forecaster=ForecasterArtifactPaths(checkpoint_id="c1"),
    )
    m.assert_matches_decision_symbol("BTC-USD")
    with pytest.raises(ValueError, match="does not match"):
        m.assert_matches_decision_symbol("ETH-USD")


def test_registry_atomic_round_trip(tmp_path: Path) -> None:
    m = AssetModelManifestV1(
        manifest_id="r1",
        symbol="SOL-USD",
        forecaster=ForecasterArtifactPaths(weights_npz_path="/tmp/w.npz"),
    )
    path = reg.upsert_manifest(tmp_path, m)
    assert path.is_file()
    loaded = reg.read_manifest(tmp_path, "SOL-USD")
    assert loaded is not None
    assert loaded.symbol == "SOL-USD"
    assert loaded.forecaster.weights_npz_path == "/tmp/w.npz"
    listed = reg.list_manifests(tmp_path)
    assert len(listed) == 1
    assert listed[0].manifest_id == "r1"


def test_resolve_manifest_single_file(tmp_path: Path) -> None:
    m = AssetModelManifestV1(manifest_id="s1", symbol="BTC-USD")
    p = tmp_path / "m.json"
    p.write_text(json.dumps(m.model_dump(mode="json")), encoding="utf-8")
    settings = AppSettings(asset_model_manifest_path=str(p))
    assert reg.resolve_manifest_for_symbol(settings, "BTC-USD") is not None


def test_pipeline_abstains_on_manifest_symbol_mismatch(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    m = AssetModelManifestV1(manifest_id="bad-bind", symbol="ETH-USD")
    p = tmp_path / "m.json"
    p.write_text(json.dumps(m.model_dump(mode="json")), encoding="utf-8")
    settings = AppSettings(asset_model_manifest_path=str(p))
    pipeline_mod._serving_mode_logged = False
    caplog.set_level(logging.ERROR)
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    _, _, route, proposal = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("methodology") == "abstain"
    assert pipe.last_forecast_packet.forecast_diagnostics.get("reason") == "manifest_symbol_mismatch"
    assert any("manifest binding failed" in r.message for r in caplog.records)
    assert route.route_id.value == "NO_TRADE"
    assert proposal is None


def test_pipeline_manifest_id_in_diagnostics_when_match(tmp_path: Path) -> None:
    m = AssetModelManifestV1(manifest_id="ok-1", symbol="BTC-USD")
    p = tmp_path / "m.json"
    p.write_text(json.dumps(m.model_dump(mode="json")), encoding="utf-8")
    settings = AppSettings(asset_model_manifest_path=str(p))
    pipeline_mod._serving_mode_logged = False
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("asset_manifest_id") == "ok-1"

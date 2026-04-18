"""FB-AP-003 / FB-AP-004: per-symbol forecaster and policy binding in DecisionPipeline."""

from __future__ import annotations

import logging

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.decisions import RouteId
from app.contracts.risk import RiskState
from app.runtime import asset_model_registry as reg
from decision_engine import pipeline as pipeline_mod
from decision_engine.pipeline import DecisionPipeline, resolve_serving_paths


def _features(close: float = 50_000.0) -> dict[str, float]:
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = close
    feats["volume"] = 1e6
    return feats


def test_multi_symbol_global_paths_without_manifest_abstains(tmp_path, monkeypatch, caplog) -> None:
    """Global NM_MODELS_* files must not serve every symbol when multiple symbols are configured."""
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    fw = tmp_path / "f.npz"
    fw.write_bytes(b"x" * 64)
    settings = AppSettings(
        market_data_symbols=["BTC-USD", "ETH-USD"],
        models_forecaster_weights_path=str(fw),
    )
    rp = resolve_serving_paths("BTC-USD", settings)
    assert rp.binding_abstain is True
    assert rp.forecaster_weights_path is None

    pipeline_mod._serving_mode_logged = False
    caplog.set_level(logging.ERROR)
    pipe = DecisionPipeline(settings=settings)
    risk = RiskState()
    _, fc, route, proposal, _ = pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe.last_forecast_packet is not None
    assert pipe.last_forecast_packet.forecast_diagnostics.get("binding_abstain") is True
    assert fc.uncertainty == 1.0
    assert route.route_id == RouteId.NO_TRADE
    assert proposal is None
    assert "FB-AP-003/004" in caplog.text


def test_multi_symbol_uses_manifest_only_no_cross_symbol_weights(
    tmp_path, monkeypatch
) -> None:
    """With manifest, paths come from manifest only — not from another symbol's global env."""
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    btc_fw = tmp_path / "btc.npz"
    btc_fw.write_bytes(b"x" * 64)
    eth_fw = tmp_path / "eth.npz"
    eth_fw.write_bytes(b"y" * 64)
    reg.save_manifest(
        AssetModelManifest(
            canonical_symbol="BTC-USD",
            forecaster_weights_path=str(btc_fw),
        )
    )
    reg.save_manifest(
        AssetModelManifest(
            canonical_symbol="ETH-USD",
            forecaster_weights_path=str(eth_fw),
        )
    )
    settings = AppSettings(
        market_data_symbols=["BTC-USD", "ETH-USD"],
        models_forecaster_weights_path=str(eth_fw),
    )
    rp_btc = resolve_serving_paths("BTC-USD", settings)
    assert not rp_btc.binding_abstain
    assert rp_btc.forecaster_weights_path == str(btc_fw)

    rp_eth = resolve_serving_paths("ETH-USD", settings)
    assert rp_eth.forecaster_weights_path == str(eth_fw)


def test_single_symbol_still_allows_global_paths(tmp_path) -> None:
    fw = tmp_path / "f.npz"
    fw.write_bytes(b"x" * 64)
    settings = AppSettings(
        market_data_symbols=["BTC-USD"],
        models_forecaster_weights_path=str(fw),
    )
    rp = resolve_serving_paths("BTC-USD", settings)
    assert not rp.binding_abstain
    assert rp.forecaster_weights_path == str(fw)

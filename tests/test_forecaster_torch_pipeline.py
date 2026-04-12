"""FB-FR-P0: distilled PyTorch forecaster → DecisionPipeline → ForecastPacket."""

from __future__ import annotations

import importlib.util

import pytest

from app.config.settings import AppSettings
from app.contracts.risk import RiskState
from decision_engine import pipeline as pipeline_mod
from decision_engine.pipeline import DecisionPipeline

_TORCH = importlib.util.find_spec("torch") is not None


@pytest.mark.skipif(not _TORCH, reason="torch optional")
def test_distilled_torch_checkpoint_pipeline_step(tmp_path) -> None:
    from forecaster_model.training.distill_mlp import train_distilled_mlp_forecaster

    out = train_distilled_mlp_forecaster(
        artifact_dir=tmp_path,
        epochs=2,
        steps_per_epoch=4,
        batch_size=8,
        device="cpu",
    )
    wpath = out["weights"]
    pipeline_mod._serving_mode_logged = False
    settings = AppSettings(
        market_data_symbols=["BTC-USD"],
        models_forecaster_torch_path=wpath,
    )
    pipe = DecisionPipeline(settings=settings)

    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = 50_000.0
    feats["volume"] = 1e6
    risk = RiskState()
    pipe.step(
        "BTC-USD",
        feats,
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
    )
    assert pipe._torch_model is not None
    pkt = pipe.last_forecast_packet
    assert pkt is not None
    assert pkt.forecast_diagnostics.get("methodology") == "pytorch_mlp"
    assert len(pkt.q_med) >= 1


@pytest.mark.skipif(not _TORCH, reason="torch optional")
def test_load_torch_forecaster_roundtrip_state_dict(tmp_path) -> None:
    from forecaster_model.training.distill_mlp import train_distilled_mlp_forecaster
    from forecaster_model.inference.torch_infer import load_torch_forecaster_checkpoint

    train_distilled_mlp_forecaster(artifact_dir=tmp_path, epochs=1, steps_per_epoch=2, device="cpu")
    pt = tmp_path / "forecaster_torch.pt"
    m, dev, cfg = load_torch_forecaster_checkpoint(pt)
    assert m is not None
    assert cfg.history_length >= 16

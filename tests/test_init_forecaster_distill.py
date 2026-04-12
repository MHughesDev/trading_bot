"""Tests for FB-AP-010 init forecaster distill helper."""

from __future__ import annotations

from pathlib import Path

from app.config.settings import AppSettings
from orchestration.init_forecaster_distill import (
    init_forecaster_detail_payload,
    run_init_forecaster_distill,
)


def test_init_forecaster_detail_payload() -> None:
    d = init_forecaster_detail_payload(
        {
            "symbol": "BTC-USD",
            "trainer": "train_distilled_mlp_forecaster",
            "methodology": "distill_mlp_synthetic_teacher",
            "epochs": 2,
            "forecaster_torch": "/tmp/x.pt",
            "forecaster_dir": "/tmp/f",
        }
    )
    assert d["symbol"] == "BTC-USD"
    assert d["epochs"] == 2


def test_run_init_forecaster_distill_writes_artifacts(tmp_path: Path) -> None:
    try:
        import torch  # noqa: F401
    except ImportError:
        import pytest

        pytest.skip("torch not installed")

    s = AppSettings(asset_init_forecaster_distill_epochs=1, models_torch_device="cpu")
    out = run_init_forecaster_distill(
        run_dir=tmp_path,
        settings=s,
        symbol="ETH-USD",
    )
    assert out["symbol"] == "ETH-USD"
    assert (tmp_path / "forecaster" / "forecaster_torch.pt").exists()

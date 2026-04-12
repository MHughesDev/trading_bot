"""Tests for per-asset init pipeline (FB-AP-006+) including FB-AP-009 features step."""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import polars as pl
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_lifecycle_state as lc
from app.runtime import asset_model_registry as reg
from control_plane import api
from orchestration.asset_init_pipeline import reset_asset_init_pipeline_for_tests


def _two_bars_df() -> pl.DataFrame:
    from datetime import UTC, datetime, timedelta

    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    return pl.DataFrame(
        {
            "timestamp": [t0, t0 + timedelta(minutes=1)],
            "open": [1.0, 1.01],
            "high": [1.0, 1.02],
            "low": [1.0, 1.0],
            "close": [1.0, 1.01],
            "volume": [1.0, 1.0],
        }
    )


def test_post_assets_init_and_poll_status(monkeypatch, tmp_path: Path) -> None:
    manifest_dir = tmp_path / "manifests"
    manifest_dir.mkdir(parents=True, exist_ok=True)
    lifecycle_dir = tmp_path / "lifecycle"
    lifecycle_dir.mkdir(parents=True, exist_ok=True)

    def fake_load_settings() -> AppSettings:
        return AppSettings(
            control_plane_api_key=None,
            asset_init_artifacts_dir=tmp_path,
            asset_init_bootstrap_lookback_days=7,
            asset_init_bootstrap_granularity_seconds=60,
        )

    monkeypatch.setattr("app.config.settings.load_settings", fake_load_settings)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", manifest_dir)
    monkeypatch.setattr(lc, "_DEFAULT_DIR", lifecycle_dir)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    reset_asset_init_pipeline_for_tests()

    monkeypatch.setattr(
        "orchestration.init_kraken_historical.fetch_symbol_bars_sync",
        lambda *_a, **_k: _two_bars_df(),
    )

    def fake_distill(*, run_dir: Path, settings: AppSettings, symbol: str) -> dict:
        fd = run_dir / "forecaster"
        fd.mkdir(parents=True, exist_ok=True)
        (fd / "forecaster_torch.pt").write_bytes(b"fake")
        return {
            "symbol": symbol,
            "trainer": "train_distilled_mlp_forecaster",
            "methodology": "distill_mlp_synthetic_teacher",
            "epochs": settings.asset_init_forecaster_distill_epochs,
            "forecaster_dir": str(fd.resolve()),
            "forecaster_torch": str((fd / "forecaster_torch.pt").resolve()),
            "forecaster_train_meta": str((fd / "forecaster_train_meta.json").resolve()),
        }

    monkeypatch.setattr(
        "orchestration.init_forecaster_distill.run_init_forecaster_distill",
        fake_distill,
    )

    client = TestClient(api.app)
    r = client.post("/assets/init/BTC-USD")
    assert r.status_code == 200
    job_id = r.json()["job_id"]

    for _ in range(50):
        st = client.get(f"/assets/init/jobs/{job_id}")
        assert st.status_code == 200
        if st.json().get("status") in ("succeeded", "failed"):
            break
        time.sleep(0.05)
    body = st.json()
    assert body["status"] == "succeeded"
    steps = body["steps"]
    assert steps[0]["step"] == "kraken_fetch"
    assert steps[0]["status"] == "done"
    assert "XBTUSD" in (steps[0].get("detail") or "")
    assert steps[1]["step"] == "validate"
    assert steps[1]["status"] == "done"
    assert "meta=" in (steps[1].get("detail") or "")
    assert steps[2]["step"] == "features"
    assert steps[2]["status"] == "done"
    meta = (steps[2].get("detail") or "").split("meta=", 1)[1]
    payload = json.loads(meta)
    assert payload.get("features_rows") == 2
    assert "schema_fingerprint" in payload
    feat_path = Path(payload["features_parquet"])
    assert feat_path.exists()
    assert steps[3]["step"] == "forecaster_train"
    assert steps[3]["status"] == "done"
    fmeta = (steps[3].get("detail") or "").split("meta=", 1)[1]
    assert json.loads(fmeta).get("methodology") == "distill_mlp_synthetic_teacher"
    assert steps[4]["step"] == "rl_init"
    assert steps[4]["status"] == "done"
    assert "policy_mlp_path" in (steps[4].get("detail") or "")
    assert steps[5]["step"] == "register"
    assert steps[5]["status"] == "done"
    rmeta = (steps[5].get("detail") or "").split("meta=", 1)[1]
    reg_payload = json.loads(rmeta)
    assert reg_payload.get("manifest_path")
    assert (Path(reg_payload["manifest_path"])).is_file()
    loaded = reg.load_manifest("BTC-USD")
    assert loaded is not None
    assert loaded.forecaster_torch_path
    assert loaded.policy_mlp_path
    assert lc.effective_lifecycle_state("BTC-USD").value == "initialized_not_active"
    reset_asset_init_pipeline_for_tests()


def test_second_init_returns_409_when_busy(monkeypatch, tmp_path: Path) -> None:
    def fake_load_settings() -> AppSettings:
        return AppSettings(
            control_plane_api_key=None,
            asset_init_artifacts_dir=tmp_path,
            asset_init_bootstrap_granularity_seconds=60,
        )

    monkeypatch.setattr("app.config.settings.load_settings", fake_load_settings)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    reset_asset_init_pipeline_for_tests()
    hold = threading.Event()

    def slow_fetch(*_a, **_k):
        hold.wait(timeout=5.0)
        return _two_bars_df()

    monkeypatch.setattr(
        "orchestration.init_kraken_historical.fetch_symbol_bars_sync",
        slow_fetch,
    )

    client = TestClient(api.app)
    r1 = client.post("/assets/init/BTC-USD")
    assert r1.status_code == 200
    try:
        r2 = client.post("/assets/init/ETH-USD")
        assert r2.status_code == 409
    finally:
        hold.set()
        time.sleep(0.2)
    reset_asset_init_pipeline_for_tests()

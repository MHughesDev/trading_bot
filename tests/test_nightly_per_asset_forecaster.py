"""FB-AP-036: nightly forecaster run per initialized asset."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.config.settings import AppSettings
from orchestration.nightly_retrain import run_nightly_training_job


def test_run_nightly_per_asset_when_manifests_exist(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls: list[tuple[str, Path]] = []

    def fake_campaign(**kwargs: object) -> dict:
        sym = kwargs.get("symbol")
        ad = kwargs.get("artifact_dir")
        calls.append((str(sym), Path(ad)))
        return {"ok": True, "symbol": sym}

    monkeypatch.setattr("orchestration.nightly_retrain.run_training_campaign", fake_campaign)
    monkeypatch.setattr("orchestration.nightly_retrain.list_manifest_symbols", lambda: ["A", "B"])
    s = AppSettings(scheduler_nightly_per_asset_forecaster=True)
    r = run_nightly_training_job(
        settings=s,
        artifact_dir=tmp_path,
        lookback_days=7,
    )
    assert r["per_asset"] is True
    assert set(r["symbols"]) == {"A", "B"}
    assert len(calls) == 2
    assert calls[0][0] == "A"
    assert calls[1][0] == "B"
    assert "nightly/A" in str(calls[0][1])
    assert "nightly/B" in str(calls[1][1])


def test_run_nightly_skips_when_no_manifests(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("orchestration.nightly_retrain.list_manifest_symbols", lambda: [])
    s = AppSettings(scheduler_nightly_per_asset_forecaster=True)
    r = run_nightly_training_job(settings=s, artifact_dir=tmp_path)
    assert r.get("skipped") is True
    assert r.get("reason") == "no_manifest_symbols"


def test_legacy_single_symbol_when_per_asset_disabled(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fake_campaign(**kwargs: object) -> dict:
        assert kwargs.get("symbol") is None
        return {"legacy": True}

    monkeypatch.setattr("orchestration.nightly_retrain.run_training_campaign", fake_campaign)
    s = AppSettings(scheduler_nightly_per_asset_forecaster=False)
    r = run_nightly_training_job(settings=s, artifact_dir=tmp_path)
    assert r.get("legacy") is True

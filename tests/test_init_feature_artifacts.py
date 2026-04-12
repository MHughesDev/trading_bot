"""Tests for FB-AP-009 init feature Parquet artifacts."""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path

import polars as pl

from app.config.settings import AppSettings
from orchestration.init_feature_artifacts import (
    init_features_detail_payload,
    write_init_feature_artifacts,
)


def test_write_init_feature_artifacts_creates_parquet_and_manifest(tmp_path: Path) -> None:
    t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    bars = pl.DataFrame(
        {
            "timestamp": [t0, t0 + timedelta(minutes=1)],
            "open": [1.0, 1.01],
            "high": [1.0, 1.02],
            "low": [1.0, 1.0],
            "close": [1.0, 1.01],
            "volume": [1.0, 1.0],
        }
    )
    s = AppSettings(
        asset_init_artifacts_dir=tmp_path,
        features_return_windows=[1, 3],
        features_volatility_windows=[5],
    )
    bp, fp, manifest = write_init_feature_artifacts(
        symbol="BTC-USD",
        job_id="test-job-id",
        cleaned_bars=bars,
        settings=s,
    )
    assert bp.exists()
    assert fp.exists()
    man_path = Path(manifest["features_parquet"]).parent / "feature_manifest.json"
    assert man_path.exists()
    loaded = json.loads(man_path.read_text(encoding="utf-8"))
    assert loaded["symbol"] == "BTC-USD"
    assert loaded["job_id"] == "test-job-id"
    assert loaded["features_rows"] == 2
    d = init_features_detail_payload(manifest)
    assert d["schema_fingerprint"] == manifest["schema_fingerprint"]
    assert "features_parquet" in d

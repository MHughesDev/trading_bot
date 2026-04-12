"""
Feature enrichment and Parquet artifacts for per-asset init (FB-AP-009).

Writes under ``settings.asset_init_artifacts_dir / <SYMBOL> / init_<job_id>/``:

- ``bars_clean.parquet`` — validated OHLCV (optional; for training/debug)
- ``features_enriched.parquet`` — :class:`data_plane.features.pipeline.FeaturePipeline` output
  with ``symbol`` column set for single-asset traceability
- ``feature_manifest.json`` — row/column counts, schema hash, paths (JSON)
"""

from __future__ import annotations

import hashlib
import json
import logging
from pathlib import Path
from typing import Any

import polars as pl

from app.config.settings import AppSettings
from app.runtime import user_data_paths as user_paths
from data_plane.features.pipeline import FeaturePipeline
from data_plane.storage.canonical_bars import CANONICAL_BAR_PARQUET_COLUMNS

logger = logging.getLogger(__name__)


def _artifact_run_dir(base: Path, symbol: str, job_id: str) -> Path:
    safe = symbol.strip().replace("/", "_")
    return base / safe / f"init_{job_id}"


def init_artifact_run_dir(settings: AppSettings, symbol: str, job_id: str) -> Path:
    """Directory for one init job (features, forecaster, …): ``<artifacts_dir>/<symbol>/init_<job_id>/``."""
    base = user_paths.asset_init_artifacts_base(Path(settings.asset_init_artifacts_dir))
    return _artifact_run_dir(base, symbol, job_id)


def write_init_feature_artifacts(
    *,
    symbol: str,
    job_id: str,
    cleaned_bars: pl.DataFrame,
    settings: AppSettings,
    feature_pipeline: FeaturePipeline | None = None,
    canonical_interval_seconds: int | None = None,
) -> tuple[Path, Path, dict[str, Any]]:
    """
    Enrich bars with ``FeaturePipeline``, persist Parquet + manifest.

    Returns:
        (bars_path, features_path, manifest_dict)
    """
    base = user_paths.asset_init_artifacts_base(Path(settings.asset_init_artifacts_dir))
    run_dir = _artifact_run_dir(base, symbol, job_id)
    run_dir.mkdir(parents=True, exist_ok=True)

    sym = symbol.strip()
    gran = canonical_interval_seconds
    if gran is None:
        if "interval_seconds" in cleaned_bars.columns:
            gran = int(cleaned_bars["interval_seconds"][0])
        else:
            raise ValueError("cleaned_bars must include interval_seconds or pass canonical_interval_seconds")
    bars_with_sym = cleaned_bars.with_columns(pl.lit(sym).alias("symbol"))
    # FB-AP-014: enforce column order for canonical Parquet
    ordered = [c for c in CANONICAL_BAR_PARQUET_COLUMNS if c in bars_with_sym.columns]
    bars_canonical = bars_with_sym.select(ordered)
    bars_path = run_dir / "bars_clean.parquet"
    bars_canonical.write_parquet(bars_path)

    fp = feature_pipeline or FeaturePipeline(
        return_windows=settings.features_return_windows,
        volatility_windows=settings.features_volatility_windows,
    )
    enriched = fp.enrich_bars(bars_with_sym)
    if enriched.height == 0:
        raise ValueError("feature enrichment produced empty frame")

    features_path = run_dir / "features_enriched.parquet"
    enriched.write_parquet(features_path)

    cols = enriched.columns
    schema_fingerprint = hashlib.sha256(",".join(sorted(cols)).encode()).hexdigest()[:16]
    manifest: dict[str, Any] = {
        "symbol": sym,
        "job_id": job_id,
        "canonical_interval_seconds": gran,
        "bars_rows": int(bars_with_sym.height),
        "features_rows": int(enriched.height),
        "feature_columns": cols,
        "n_feature_columns": len(cols),
        "schema_fingerprint": schema_fingerprint,
        "bars_parquet": str(bars_path.resolve()),
        "features_parquet": str(features_path.resolve()),
        "return_windows": list(settings.features_return_windows),
        "volatility_windows": list(settings.features_volatility_windows),
    }
    man_path = run_dir / "feature_manifest.json"
    man_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    logger.info(
        "init features symbol=%s job_id=%s rows=%s cols=%s dir=%s",
        sym,
        job_id,
        enriched.height,
        len(cols),
        run_dir,
    )
    return bars_path, features_path, manifest


def init_features_detail_payload(manifest: dict[str, Any]) -> dict[str, Any]:
    """Subset for job step ``meta=`` JSON (avoid huge column lists)."""
    return {
        "symbol": manifest.get("symbol"),
        "canonical_interval_seconds": manifest.get("canonical_interval_seconds"),
        "bars_rows": manifest.get("bars_rows"),
        "features_rows": manifest.get("features_rows"),
        "n_feature_columns": manifest.get("n_feature_columns"),
        "schema_fingerprint": manifest.get("schema_fingerprint"),
        "features_parquet": manifest.get("features_parquet"),
        "bars_parquet": manifest.get("bars_parquet"),
    }

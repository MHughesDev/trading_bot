"""Shared Polars feature frame → last row dict (live rolling bars + replay)."""

from __future__ import annotations

from typing import Any

import polars as pl

from data_plane.features.canonical_normalize import (
    bar_age_seconds_from_timestamp,
    normalize_feature_row,
)
from data_plane.features.pipeline import FeaturePipeline

FEATURE_SCHEMA_VERSION = 1


def enrich_bars_last_row(
    bars: pl.DataFrame,
    feature_pipeline: FeaturePipeline,
    *,
    bar_timestamp: Any | None = None,
    stale_data_seconds: float = 120.0,
) -> dict[str, float] | None:
    """Run FeaturePipeline.enrich_bars and return the last row as float dict (+ schema_version)."""
    if bars.height == 0:
        return None
    enriched = feature_pipeline.enrich_bars(bars)
    last = enriched.tail(1)
    row = last.to_dicts()[0]
    out: dict[str, float] = {}
    ts = row.get("timestamp")
    for k, v in row.items():
        if k == "timestamp":
            continue
        if isinstance(v, (float, int)):
            out[str(k)] = float(v)
        elif v is not None and hasattr(v, "item"):
            try:
                out[str(k)] = float(v.item())
            except (TypeError, ValueError):
                pass
    out["feature_schema_version"] = float(FEATURE_SCHEMA_VERSION)

    age = None
    ref_ts = bar_timestamp if bar_timestamp is not None else ts
    if ref_ts is not None:
        age = bar_age_seconds_from_timestamp(ref_ts)
    out = normalize_feature_row(out, bar_age_seconds=age, stale_data_seconds=stale_data_seconds)
    return out


def merge_feature_overlays(base: dict[str, float], overlays: dict[str, float] | None) -> dict[str, float]:
    """Microstructure / memory keys from tick overlay win on key collision."""
    if not overlays:
        return base
    merged = dict(base)
    merged.update(overlays)
    return merged

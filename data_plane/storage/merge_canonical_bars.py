"""
Idempotent merge / deduplication for canonical OHLCV bars (FB-AP-015).

**Key:** ``(symbol, timestamp, interval_seconds)`` — matches QuestDB ``canonical_bars`` and
:class:`app.contracts.events.BarEvent`.

**Conflict rule:** **last-write-wins** — when duplicate keys exist after ``concat``, the last row
in frame order wins (same as ``bootstrap_bars`` ``unique(..., keep="last")``).

Use for backfill replays, merging REST batches, or combining Parquet chunks before persist.
"""

from __future__ import annotations

import polars as pl

# Polars column names (timestamp = bucket start UTC).
DEDUP_KEY_COLUMNS: tuple[str, ...] = ("symbol", "timestamp", "interval_seconds")


def dedupe_canonical_bars_last_wins(df: pl.DataFrame) -> pl.DataFrame:
    """
    Drop duplicate keys, keeping the **last** occurrence (replay-safe last-write-wins).

    Raises:
        ValueError: if required columns are missing.
    """
    missing = [c for c in DEDUP_KEY_COLUMNS if c not in df.columns]
    if missing:
        raise ValueError(f"canonical bars frame missing columns: {missing}")
    if df.height == 0:
        return df
    return df.unique(subset=list(DEDUP_KEY_COLUMNS), keep="last").sort("timestamp")


def merge_canonical_bars_frames(*frames: pl.DataFrame, sort: bool = True) -> pl.DataFrame:
    """
    Concatenate one or more frames and apply :func:`dedupe_canonical_bars_last_wins`.

    Empty inputs return an empty frame with no schema inference — pass at least one non-empty
    frame or build schema from :data:`data_plane.storage.canonical_bars.CANONICAL_BAR_PARQUET_COLUMNS`.
    """
    if not frames:
        raise ValueError("merge_canonical_bars_frames requires at least one frame")
    non_empty = [f for f in frames if f.height > 0]
    if not non_empty:
        return frames[0].clear()
    out = pl.concat(non_empty, how="vertical")
    out = dedupe_canonical_bars_last_wins(out)
    if sort:
        out = out.sort("timestamp")
    return out

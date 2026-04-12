"""
Startup Kraken REST backfill into QuestDB ``canonical_bars`` (FB-AP-019).

Uses :func:`data_plane.storage.startup_gap_detection.detect_canonical_bar_gaps` (FB-AP-018) and
idempotent :meth:`data_plane.storage.questdb.QuestDBWriter.insert_bar` (FB-AP-015).
"""

from __future__ import annotations

import logging
from datetime import UTC, datetime, timedelta

from app.config.settings import AppSettings
from app.contracts.events import BarEvent
from app.runtime.canonical_bar_watermark import write_canonical_through
from data_plane.bootstrap_bars import validate_and_clean_init_bootstrap_bars
from data_plane.storage.questdb import QuestDBWriter
from data_plane.storage.startup_gap_detection import CanonicalBarGap
from orchestration.real_data_bars import fetch_symbol_bars_async

logger = logging.getLogger(__name__)


def _align_bucket_start(ts: datetime, interval_seconds: int) -> datetime:
    e = int(ts.astimezone(UTC).timestamp())
    floored = e - (e % interval_seconds)
    return datetime.fromtimestamp(floored, tz=UTC)


async def backfill_gap_to_questdb(
    cfg: AppSettings,
    qdb: QuestDBWriter,
    gap: CanonicalBarGap,
    *,
    max_lookback_days: int,
) -> dict[str, object]:
    """
    Fetch Kraken history for ``gap`` range, validate, insert bars, update watermark.

    Returns a small summary dict for logging.
    """
    sym = gap.symbol
    bar_sec = gap.interval_seconds
    last_closed = gap.last_closed_bucket_start
    fetch_end = last_closed + timedelta(seconds=bar_sec)

    if gap.max_stored_ts is None:
        lookback = max(1, int(max_lookback_days))
        raw_start = last_closed - timedelta(days=lookback)
        fetch_start = _align_bucket_start(raw_start, bar_sec)
    else:
        if not gap.gap_start:
            return {"symbol": sym, "skipped": True, "reason": "no_gap_start"}
        fetch_start = gap.gap_start

    if fetch_start >= fetch_end:
        return {"symbol": sym, "skipped": True, "reason": "empty_range"}

    try:
        df = await fetch_symbol_bars_async(
            sym,
            fetch_start,
            fetch_end,
            granularity_seconds=bar_sec,
        )
    except Exception:
        logger.exception("kraken backfill fetch failed symbol=%s", sym)
        return {"symbol": sym, "error": "fetch_failed"}

    if df.height == 0:
        logger.warning("kraken backfill: no rows symbol=%s", sym)
        return {"symbol": sym, "rows": 0}

    try:
        cleaned = validate_and_clean_init_bootstrap_bars(df, granularity_seconds=bar_sec)
    except Exception:
        logger.exception("kraken backfill validate failed symbol=%s", sym)
        return {"symbol": sym, "error": "validate_failed"}

    work = cleaned.cleaned
    inserted = 0
    max_ts: datetime | None = None
    for row in work.iter_rows(named=True):
        ts = row["timestamp"]
        if isinstance(ts, datetime):
            ts_dt = ts.astimezone(UTC) if ts.tzinfo else ts.replace(tzinfo=UTC)
        else:
            continue
        if ts_dt < fetch_start or ts_dt >= fetch_end:
            continue
        try:
            bar = BarEvent(
                timestamp=ts_dt,
                symbol=sym,
                open=float(row["open"]),
                high=float(row["high"]),
                low=float(row["low"]),
                close=float(row["close"]),
                volume=float(row["volume"]),
                interval_seconds=bar_sec,
                source="kraken",
                schema_version=1,
            )
            await qdb.insert_bar(bar)
            inserted += 1
            if max_ts is None or ts_dt > max_ts:
                max_ts = ts_dt
        except Exception:
            logger.exception("kraken backfill insert_bar failed symbol=%s ts=%s", sym, ts_dt)

    if max_ts is not None:
        try:
            write_canonical_through(sym, canonical_through_ts=max_ts, interval_seconds=bar_sec)
        except Exception:
            logger.exception("watermark write failed symbol=%s", sym)

    return {
        "symbol": sym,
        "rows_fetched": int(df.height),
        "rows_cleaned": int(work.height),
        "inserted": inserted,
        "canonical_through_ts": max_ts.isoformat() if max_ts else None,
    }


async def run_startup_canonical_backfill(
    cfg: AppSettings,
    qdb: QuestDBWriter,
    *,
    gaps: list[CanonicalBarGap],
    max_lookback_days: int | None = None,
) -> list[dict[str, object]]:
    """Backfill every gap with ``gap_detected``."""
    mld = max_lookback_days if max_lookback_days is not None else cfg.questdb_backfill_max_lookback_days
    out: list[dict[str, object]] = []
    for g in gaps:
        if not g.gap_detected:
            continue
        summary = await backfill_gap_to_questdb(cfg, qdb, g, max_lookback_days=mld)
        out.append(summary)
        logger.info("startup_canonical_backfill %s", summary)
    return out

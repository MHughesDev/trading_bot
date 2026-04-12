"""HTTP fetch for Asset chart bars (FB-AP-028) — no Streamlit."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

import httpx

from app.config.settings import load_settings
from control_plane.asset_chart_data import (
    Preset,
    bars_dicts_from_df,
    preset_to_request,
    resample_monthly_ohlc,
    resample_ohlc,
    resample_weekly_ohlc,
    _bars_payload_to_df,
)
from control_plane.streamlit_util import get_api_base
from data_plane.bars.derived_intervals import daily_ohlc_aligned_with_training


def _training_granularity_seconds() -> int:
    """Match ``AppSettings.training_data_granularity_seconds`` (FB-AP-017 chart vs training)."""
    return max(1, int(load_settings().training_data_granularity_seconds))


def _get_chart_json(params: dict[str, Any], *, timeout: float = 45.0) -> dict[str, Any]:
    base = get_api_base()
    r = httpx.get(f"{base}/assets/chart/bars", params=params, timeout=timeout)
    r.raise_for_status()
    return r.json()


def _get_markers_json(params: dict[str, Any], *, timeout: float = 30.0) -> dict[str, Any]:
    base = get_api_base()
    r = httpx.get(f"{base}/assets/chart/trade-markers", params=params, timeout=timeout)
    r.raise_for_status()
    return r.json()


def chart_time_window(
    preset: Preset,
    *,
    now: datetime | None = None,
) -> tuple[datetime, datetime]:
    """Same UTC window as ``fetch_bars_for_preset`` (for marker overlay alignment)."""
    t_end = (now or datetime.now(UTC)).astimezone(UTC)
    req = preset_to_request(preset)
    return t_end - req.window, t_end


def fetch_trade_markers_for_window(
    symbol: str,
    start: datetime,
    end: datetime,
    *,
    limit: int = 2000,
) -> list[dict[str, Any]]:
    """FB-AP-029: ``GET /assets/chart/trade-markers`` for chart overlay."""
    sym = symbol.strip()
    data = _get_markers_json(
        {
            "symbol": sym,
            "start": start.isoformat(),
            "end": end.isoformat(),
            "limit": min(int(limit), 10_000),
        }
    )
    return list(data.get("markers") or [])


def fetch_bars_for_preset(
    symbol: str,
    preset: Preset,
    *,
    now: datetime | None = None,
) -> tuple[list[dict[str, Any]], str, int]:
    """
    Load OHLC bars for ``symbol`` and UI preset.

    Returns ``(bars, note, effective_interval_seconds)``.
    """
    sym = symbol.strip()
    t_end = (now or datetime.now(UTC)).astimezone(UTC)
    req = preset_to_request(preset)
    t_start = t_end - req.window

    def _pull(p: dict[str, Any]) -> list[dict[str, Any]]:
        data = _get_chart_json(p)
        return list(data.get("bars") or [])

    # Week / month: derive from **daily** bars built from canonical seconds when possible,
    # using the same training granularity as ``NM_TRAINING_DATA_GRANULARITY_SECONDS`` (FB-AP-017).
    g_train = _training_granularity_seconds()
    if preset == "week":
        p_1s = {
            "symbol": sym,
            "start": t_start.isoformat(),
            "end": t_end.isoformat(),
            "limit": min(50_000, max(req.limit, 8_000)),
            "interval_seconds": 1,
        }
        fine = _pull(p_1s)
        if fine:
            df_daily = daily_ohlc_aligned_with_training(
                _bars_payload_to_df(fine),
                training_granularity_seconds=g_train,
            )
            df = resample_weekly_ohlc(df_daily)
            note = (
                f"weekly OHLC (from 1s canonical → daily @ {g_train}s step → week)"
                if g_train > 1
                else "weekly OHLC (from 1s canonical → daily → week)"
            )
            return bars_dicts_from_df(df), note, 604_800
        p_d = {
            "symbol": sym,
            "start": t_start.isoformat(),
            "end": t_end.isoformat(),
            "limit": req.limit,
            "interval_seconds": 86_400,
        }
        daily = _pull(p_d)
        if not daily:
            return [], "no daily bars in QuestDB for this window — enable canonical bar persistence / backfill", 604_800
        df = resample_weekly_ohlc(_bars_payload_to_df(daily))
        return bars_dicts_from_df(df), "weekly OHLC (aggregated from stored daily bars)", 604_800

    if preset == "month":
        p_1s = {
            "symbol": sym,
            "start": t_start.isoformat(),
            "end": t_end.isoformat(),
            "limit": min(50_000, max(req.limit, 8_000)),
            "interval_seconds": 1,
        }
        fine = _pull(p_1s)
        if fine:
            df_daily = daily_ohlc_aligned_with_training(
                _bars_payload_to_df(fine),
                training_granularity_seconds=g_train,
            )
            df = resample_monthly_ohlc(df_daily)
            note = (
                f"monthly OHLC (from 1s canonical → daily @ {g_train}s step → month)"
                if g_train > 1
                else "monthly OHLC (from 1s canonical → daily → month)"
            )
            return bars_dicts_from_df(df), note, 2_592_000
        p_d = {
            "symbol": sym,
            "start": t_start.isoformat(),
            "end": t_end.isoformat(),
            "limit": req.limit,
            "interval_seconds": 86_400,
        }
        daily = _pull(p_d)
        if not daily:
            return [], "no daily bars in QuestDB for this window — enable canonical bar persistence / backfill", 2_592_000
        df = resample_monthly_ohlc(_bars_payload_to_df(daily))
        return bars_dicts_from_df(df), "monthly OHLC (aggregated from stored daily bars)", 2_592_000

    params: dict[str, Any] = {
        "symbol": sym,
        "start": t_start.isoformat(),
        "end": t_end.isoformat(),
        "limit": req.limit,
        "interval_seconds": req.interval_seconds,
    }
    bars = _pull(params)
    if bars:
        return bars, "native QuestDB interval", req.interval_seconds

    # Finer presets: try one-step coarser fetch and resample up (if finer data missing)
    if preset == "hour" and req.interval_seconds == 3600:
        p60 = {**params, "interval_seconds": 60, "limit": min(20_000, req.limit * 24)}
        m1 = _pull(p60)
        if m1:
            df = resample_ohlc(_bars_payload_to_df(m1), bucket_seconds=3600)
            return bars_dicts_from_df(df), "hourly OHLC resampled from 1m bars", 3600

    if preset == "minute" and req.interval_seconds == 60:
        p1 = {**params, "interval_seconds": 1, "limit": min(50_000, req.limit * 60)}
        s1 = _pull(p1)
        if s1:
            df = resample_ohlc(_bars_payload_to_df(s1), bucket_seconds=60)
            return bars_dicts_from_df(df), "1m OHLC resampled from 1s bars", 60

    return (
        [],
        "no rows for this symbol/window at the requested interval — check QuestDB canonical_bars",
        req.interval_seconds,
    )

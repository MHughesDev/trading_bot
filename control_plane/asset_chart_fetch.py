"""HTTP fetch for Asset chart bars (FB-AP-028) — no Streamlit."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

import httpx

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


def _get_chart_json(params: dict[str, Any], *, timeout: float = 45.0) -> dict[str, Any]:
    base = get_api_base()
    r = httpx.get(f"{base}/assets/chart/bars", params=params, timeout=timeout)
    r.raise_for_status()
    return r.json()


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

    # Week / month: always derive from daily bars (FB-AP-017-style rollup client-side)
    if preset == "week":
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
        return bars_dicts_from_df(df), "weekly OHLC (aggregated from daily bars)", 604_800

    if preset == "month":
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
        return bars_dicts_from_df(df), "monthly OHLC (aggregated from daily bars)", 2_592_000

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

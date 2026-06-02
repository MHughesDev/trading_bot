"""Shared helpers for the reusable trading chart package."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
import re
from typing import Any

import pandas as pd

TIMEFRAME_OPTIONS: tuple[str, ...] = (
    "1min",
    "5min",
    "15min",
    "30min",
    "1H",
    "1D",
    "1W",
    "1M",
    "1Y",
)

SYMBOL_PATTERN = re.compile(r"^[A-Z0-9._:/-]{2,30}$")


@dataclass(frozen=True)
class TimeframeSpec:
    """API request settings and optional client-side resample rule."""

    interval_seconds: int
    lookback: timedelta
    limit: int
    resample_rule: str | None = None


TIMEFRAME_SPECS: dict[str, TimeframeSpec] = {
    "1min": TimeframeSpec(60, timedelta(days=2), 2_880),
    "5min": TimeframeSpec(300, timedelta(days=7), 2_016),
    "15min": TimeframeSpec(900, timedelta(days=21), 2_016),
    "30min": TimeframeSpec(1_800, timedelta(days=45), 2_160),
    "1H": TimeframeSpec(3_600, timedelta(days=120), 2_880),
    "1D": TimeframeSpec(86_400, timedelta(days=730), 730),
    "1W": TimeframeSpec(86_400, timedelta(days=365 * 5), 1_825, "W"),
    "1M": TimeframeSpec(86_400, timedelta(days=365 * 10), 3_650, "MS"),
    "1Y": TimeframeSpec(86_400, timedelta(days=365 * 20), 7_300, "YS"),
}


def empty_ohlcv_frame() -> pd.DataFrame:
    """Return the canonical empty OHLCV frame."""
    return pd.DataFrame(columns=["time", "open", "high", "low", "close", "volume"])


def normalize_symbol(symbol: str) -> str:
    """Normalize a symbol for chart lookups."""
    normalized = str(symbol or "").strip().upper()
    if not SYMBOL_PATTERN.fullmatch(normalized):
        raise ValueError(f"Invalid symbol: {symbol!r}")
    return normalized


def timeframe_spec(timeframe: str) -> TimeframeSpec:
    """Return the fetch definition for a supported timeframe."""
    try:
        return TIMEFRAME_SPECS[timeframe]
    except KeyError as exc:
        raise ValueError(f"Unsupported timeframe: {timeframe!r}") from exc


def window_bounds(timeframe: str, *, now: datetime | None = None) -> tuple[datetime, datetime]:
    """Compute the UTC fetch window for a timeframe."""
    current = (now or datetime.now(UTC)).astimezone(UTC)
    spec = timeframe_spec(timeframe)
    return current - spec.lookback, current


def bars_payload_to_frame(payload: list[dict[str, Any]]) -> pd.DataFrame:
    """Convert the control-plane bars payload into the chart dataframe shape."""
    if not payload:
        return empty_ohlcv_frame()
    frame = pd.DataFrame(payload).rename(columns={"ts": "time"})
    if "time" not in frame.columns:
        return empty_ohlcv_frame()
    frame["time"] = pd.to_datetime(frame["time"], utc=True, errors="coerce")
    frame = frame.dropna(subset=["time"])
    for column in ("open", "high", "low", "close", "volume"):
        if column not in frame.columns:
            frame[column] = 0.0
        frame[column] = pd.to_numeric(frame[column], errors="coerce")
    frame = frame.dropna(subset=["open", "high", "low", "close"]).sort_values("time")
    return frame[["time", "open", "high", "low", "close", "volume"]].reset_index(drop=True)


def resample_ohlcv(frame: pd.DataFrame, rule: str) -> pd.DataFrame:
    """Resample daily candles into longer horizons."""
    if frame.empty:
        return empty_ohlcv_frame()
    aggregated = (
        frame.set_index("time")
        .resample(rule, label="left", closed="left")
        .agg(
            {
                "open": "first",
                "high": "max",
                "low": "min",
                "close": "last",
                "volume": "sum",
            }
        )
        .dropna(subset=["open", "high", "low", "close"])
        .reset_index()
    )
    return aggregated[["time", "open", "high", "low", "close", "volume"]]


def heikin_ashi(frame: pd.DataFrame) -> pd.DataFrame:
    """Return a Heikin-Ashi OHLC frame derived from a standard OHLCV frame.

    HA close = (O+H+L+C)/4; HA open = avg of prior HA open/close; HA high/low extend to the raw
    high/low. Smooths trend visualisation; volume/time columns are preserved.
    """
    if frame.empty:
        return frame
    import numpy as np

    o = frame["open"].to_numpy(dtype=float)
    h = frame["high"].to_numpy(dtype=float)
    low_ = frame["low"].to_numpy(dtype=float)
    c = frame["close"].to_numpy(dtype=float)
    n = len(c)
    ha_close = (o + h + low_ + c) / 4.0
    ha_open = np.empty(n, dtype=float)
    ha_open[0] = (o[0] + c[0]) / 2.0
    for i in range(1, n):
        ha_open[i] = (ha_open[i - 1] + ha_close[i - 1]) / 2.0
    ha_high = np.maximum.reduce([h, ha_open, ha_close])
    ha_low = np.minimum.reduce([low_, ha_open, ha_close])
    out = frame.copy()
    out["open"] = ha_open
    out["high"] = ha_high
    out["low"] = ha_low
    out["close"] = ha_close
    return out


def indicator_frame(series: pd.Series, name: str, times: pd.Series) -> pd.DataFrame:
    """Build a chart-ready indicator frame."""
    frame = pd.DataFrame({"time": times, name: pd.to_numeric(series, errors="coerce")})
    return frame.dropna(subset=[name]).reset_index(drop=True)


def compute_indicator_frames(frame: pd.DataFrame, name: str, **params: Any) -> dict[str, pd.DataFrame]:
    """Return one or more chart-ready indicator series for a registered study.

    Dispatches through :data:`charts.indicators.REGISTRY` (pure-pandas implementations); the
    returned dict maps each output series name to a ``{time, <name>}`` frame.
    """
    if frame.empty:
        return {}
    from charts.indicators import REGISTRY

    key = name.lower().replace(" ", "_")
    spec = REGISTRY.get(key)
    if spec is None:
        raise ValueError(f"Unsupported indicator: {name!r}")
    merged = {**spec.default_params, **params}
    times = frame["time"]
    return {
        series_name: indicator_frame(series, series_name, times)
        for series_name, series in spec.compute(frame, merged).items()
    }

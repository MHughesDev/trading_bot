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


def indicator_frame(series: pd.Series, name: str, times: pd.Series) -> pd.DataFrame:
    """Build a chart-ready indicator frame."""
    frame = pd.DataFrame({"time": times, name: pd.to_numeric(series, errors="coerce")})
    return frame.dropna(subset=[name]).reset_index(drop=True)


def compute_indicator_frames(frame: pd.DataFrame, name: str, **params: Any) -> dict[str, pd.DataFrame]:
    """Return one or more indicator series for a supported study."""
    if frame.empty:
        return {}

    close = frame["close"]
    high = frame["high"]
    low = frame["low"]
    volume = frame["volume"]
    times = frame["time"]
    key = name.lower().replace(" ", "_")

    try:
        import talib  # type: ignore[import-not-found]
    except ImportError:
        talib = None
    try:
        import pandas_ta as pta  # type: ignore[import-not-found]
    except ImportError:
        pta = None

    if key == "sma":
        length = int(params.get("length", 20))
        values = talib.SMA(close, timeperiod=length) if talib else close.rolling(length).mean()
        return {f"SMA {length}": indicator_frame(values, f"SMA {length}", times)}

    if key == "ema":
        length = int(params.get("length", 20))
        values = talib.EMA(close, timeperiod=length) if talib else close.ewm(span=length, adjust=False).mean()
        return {f"EMA {length}": indicator_frame(values, f"EMA {length}", times)}

    if key == "bollinger_bands":
        length = int(params.get("length", 20))
        std_dev = float(params.get("std_dev", 2.0))
        if talib:
            upper, middle, lower = talib.BBANDS(close, timeperiod=length, nbdevup=std_dev, nbdevdn=std_dev)
        elif pta:
            bands = pta.bbands(close, length=length, std=std_dev)
            lower = bands.iloc[:, 0]
            middle = bands.iloc[:, 1]
            upper = bands.iloc[:, 2]
        else:
            middle = close.rolling(length).mean()
            std = close.rolling(length).std()
            upper = middle + (std * std_dev)
            lower = middle - (std * std_dev)
        return {
            f"BB Upper {length}": indicator_frame(upper, f"BB Upper {length}", times),
            f"BB Basis {length}": indicator_frame(middle, f"BB Basis {length}", times),
            f"BB Lower {length}": indicator_frame(lower, f"BB Lower {length}", times),
        }

    if key == "vwap":
        typical_price = (high + low + close) / 3
        values = (typical_price * volume).cumsum() / volume.replace(0, pd.NA).cumsum()
        return {"VWAP": indicator_frame(values, "VWAP", times)}

    if key == "rsi":
        length = int(params.get("length", 14))
        if talib:
            values = talib.RSI(close, timeperiod=length)
        elif pta:
            values = pta.rsi(close, length=length)
        else:
            delta = close.diff()
            gain = delta.clip(lower=0).ewm(alpha=1 / length, adjust=False).mean()
            loss = (-delta.clip(upper=0)).ewm(alpha=1 / length, adjust=False).mean()
            rs = gain / loss.replace(0, pd.NA)
            values = 100 - (100 / (1 + rs))
        return {f"RSI {length}": indicator_frame(values, f"RSI {length}", times)}

    if key == "macd":
        fast = int(params.get("fast", 12))
        slow = int(params.get("slow", 26))
        signal = int(params.get("signal", 9))
        if talib:
            macd, signal_line, histogram = talib.MACD(close, fastperiod=fast, slowperiod=slow, signalperiod=signal)
        elif pta:
            values = pta.macd(close, fast=fast, slow=slow, signal=signal)
            macd = values.iloc[:, 0]
            histogram = values.iloc[:, 1]
            signal_line = values.iloc[:, 2]
        else:
            fast_ema = close.ewm(span=fast, adjust=False).mean()
            slow_ema = close.ewm(span=slow, adjust=False).mean()
            macd = fast_ema - slow_ema
            signal_line = macd.ewm(span=signal, adjust=False).mean()
            histogram = macd - signal_line
        return {
            "MACD": indicator_frame(macd, "MACD", times),
            "MACD Signal": indicator_frame(signal_line, "MACD Signal", times),
            "MACD Histogram": indicator_frame(histogram, "MACD Histogram", times),
        }

    if key == "volume":
        return {"Volume": indicator_frame(volume, "Volume", times)}

    raise ValueError(f"Unsupported indicator: {name!r}")

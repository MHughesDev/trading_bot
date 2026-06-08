"""Chart data adapters for reusable trading views."""

from __future__ import annotations

from abc import ABC, abstractmethod
import os
from typing import Final

import httpx
import pandas as pd

from charts._helpers import (
    bars_payload_to_frame,
    empty_ohlcv_frame,
    normalize_symbol,
    resample_ohlcv,
    timeframe_spec,
    window_bounds,
)

DEFAULT_API_BASE: Final[str] = "http://127.0.0.1:8000"


class OHLCVDataSource(ABC):
    """Abstract OHLCV backend used by the chart frontend."""

    @abstractmethod
    def get_ohlcv(self, symbol: str, timeframe: str) -> pd.DataFrame:
        """Return OHLCV candles with columns ``time, open, high, low, close, volume``."""


class ControlPlaneDataFeed(OHLCVDataSource):
    """Concrete adapter backed by the existing FastAPI ``/assets/chart/bars`` endpoint."""

    def __init__(self, api_base: str | None = None, timeout: float = 20.0) -> None:
        self.api_base = (api_base or os.getenv("NM_CONTROL_PLANE_URL", DEFAULT_API_BASE)).rstrip("/")
        self.timeout = timeout

    def get_ohlcv(self, symbol: str, timeframe: str) -> pd.DataFrame:
        """Fetch bars for a symbol and timeframe from the control plane."""
        normalized_symbol = normalize_symbol(symbol)
        spec = timeframe_spec(timeframe)
        start, end = window_bounds(timeframe)
        source_interval = 86_400 if spec.resample_rule else spec.interval_seconds
        payload = self._fetch_payload(
            symbol=normalized_symbol,
            interval_seconds=source_interval,
            start=start,
            end=end,
            limit=spec.limit,
        )
        frame = bars_payload_to_frame(payload)
        if frame.empty:
            return empty_ohlcv_frame()
        if spec.resample_rule:
            frame = resample_ohlcv(frame, spec.resample_rule)
        return frame[["time", "open", "high", "low", "close", "volume"]].reset_index(drop=True)

    def _fetch_payload(
        self,
        *,
        symbol: str,
        interval_seconds: int,
        start,
        end,
        limit: int,
    ) -> list[dict]:
        response = httpx.get(
            f"{self.api_base}/assets/chart/bars",
            params={
                "symbol": symbol,
                "interval_seconds": interval_seconds,
                "start": start.isoformat(),
                "end": end.isoformat(),
                "limit": limit,
            },
            timeout=self.timeout,
        )
        response.raise_for_status()
        return list(response.json().get("bars") or [])


_DEFAULT_DATA_FEED = ControlPlaneDataFeed()


def get_ohlcv(symbol: str, timeframe: str) -> pd.DataFrame:
    """Return OHLCV candles from the configured default data feed."""
    return _DEFAULT_DATA_FEED.get_ohlcv(symbol=symbol, timeframe=timeframe)

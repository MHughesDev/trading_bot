"""Rolling bars + FeaturePipeline parity with live_service tick path (microservice)."""

from __future__ import annotations

import os
from datetime import UTC, datetime
from typing import Any

from data_plane.bars.rolling import RollingBars
from data_plane.features.pipeline import FeaturePipeline
from decision_engine.feature_frame import enrich_bars_last_row, merge_feature_overlays
from shared.messaging.envelope import EventEnvelope


def _bar_interval_s() -> int:
    try:
        v = int(os.getenv("NM_FEATURE_SERVICE_BAR_INTERVAL_SECONDS", "1"))
        return max(1, v)
    except ValueError:
        return 1


class FeatureRowBuilder:
    """
    Maintains per-symbol RollingBars and applies FeaturePipeline to the latest bucket.
    """

    def __init__(self) -> None:
        self._bars: dict[str, RollingBars] = {}
        self._pipeline = FeaturePipeline()
        self._interval = _bar_interval_s()

    def _rolling(self, symbol: str) -> RollingBars:
        if symbol not in self._bars:
            self._bars[symbol] = RollingBars(symbol, interval_seconds=self._interval)
        return self._bars[symbol]

    def build_from_tick(self, env: EventEnvelope) -> dict[str, Any]:
        p = env.payload
        symbol = str(p.get("symbol", env.symbol or ""))
        mid = float(p.get("mid_price", p.get("price", 50_000.0)))
        ts_raw = p.get("data_timestamp")
        if isinstance(ts_raw, str):
            try:
                ts = datetime.fromisoformat(ts_raw.replace("Z", "+00:00"))
            except ValueError:
                ts = datetime.now(UTC)
        else:
            ts = datetime.now(UTC)
        if ts.tzinfo is None:
            ts = ts.replace(tzinfo=UTC)

        roll = self._rolling(symbol)
        roll.on_tick(mid, ts, size=0.0)
        bars = roll.bars_frame_with_partial()
        base = enrich_bars_last_row(bars, self._pipeline)
        if base is None:
            base = {}

        spread_bps = float(p.get("spread_bps", 5.0))
        spread = mid * (spread_bps / 10_000.0)
        micro = self._pipeline.microstructure(
            spread=spread,
            bid_sz=1.0,
            ask_sz=1.0,
            volume_delta=0.0,
        )
        merged = merge_feature_overlays(base, micro)

        out: dict[str, Any] = {
            "symbol": symbol,
            "direction": int(p.get("direction", 1)),
            "size_fraction": float(p.get("size_fraction", 0.1)),
            "route_id": str(p.get("route_id", "SCALPING")),
            "mid_price": mid,
            "spread_bps": spread_bps,
        }
        out.update(merged)
        return out

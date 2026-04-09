"""Build feature dict for live path from normalized WS objects + memory overlay."""

from __future__ import annotations

from typing import Any

from data_plane.features.pipeline import FeaturePipeline
from data_plane.ingest.normalizers import OrderBookLevel2Snapshot, TickerSnapshot, TradeTick


def feature_row_from_tick(
    norm: Any,
    *,
    memory: dict[str, float] | None = None,
    sentiment: dict[str, float] | None = None,
    pipeline: FeaturePipeline | None = None,
) -> dict[str, float]:
    """Scalar features for DecisionPipeline until rolling bar frame exists."""
    pipe = pipeline or FeaturePipeline()
    close = float(getattr(norm, "price", 0.0) or 0.0)
    row: dict[str, float] = {"close": close}

    if isinstance(norm, TickerSnapshot) and norm.bid is not None and norm.ask is not None:
        spread_abs = float(norm.ask) - float(norm.bid)
        mid = close if close else (float(norm.bid) + float(norm.ask)) / 2.0
        row["micro_spread_bps"] = spread_abs / mid * 10_000.0 if mid else 0.0
        micro = pipe.microstructure(spread_abs, 1.0, 1.0, 0.0)
        row.update(micro)
    elif isinstance(norm, OrderBookLevel2Snapshot) and norm.bids and norm.asks:
        bb = max(norm.bids, key=lambda x: x[0])[0]
        aa = min(norm.asks, key=lambda x: x[0])[0]
        mid = (bb + aa) / 2.0
        spread_abs = aa - bb
        row["micro_spread_bps"] = spread_abs / mid * 10_000.0 if mid else 0.0
        bid_sz = sum(s for _, s in norm.bids[:5])
        ask_sz = sum(s for _, s in norm.asks[:5])
        row.update(pipe.microstructure(spread_abs, bid_sz, ask_sz, 0.0))

    if isinstance(norm, TradeTick):
        row["last_trade_size"] = float(norm.size)

    if memory:
        row.update(memory)
    if sentiment:
        row.update(pipe.sentiment_features(**sentiment))
    else:
        row.update(pipe.sentiment_features())
    return row

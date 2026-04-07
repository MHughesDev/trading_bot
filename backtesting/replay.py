"""Replay historical bars through the same decision + risk path as live."""

from __future__ import annotations

from datetime import datetime

import polars as pl

from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def replay_decisions(
    bars: pl.DataFrame,
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    *,
    symbol: str,
    spread_bps: float = 5.0,
) -> list[dict]:
    """Iterate rows of enriched feature frame; return list of decision dicts."""
    if bars.height == 0:
        return []
    risk = RiskState()
    rows_out: list[dict] = []
    for row in bars.iter_rows(named=True):
        feats = {k: float(v) for k, v in row.items() if k != "timestamp" and isinstance(v, (int, float))}
        ts = row.get("timestamp")
        if isinstance(ts, datetime):
            dt = ts
        else:
            dt = None
        regime, fc, route, action = pipeline.step(symbol, feats, spread_bps, risk)
        trade, risk = risk_engine.evaluate(
            symbol,
            action,
            risk,
            mid_price=float(row.get("close", 1.0)),
            spread_bps=spread_bps,
            data_timestamp=dt,
        )
        rows_out.append(
            {
                "timestamp": ts,
                "route": route.route_id.value,
                "regime": regime.semantic.value,
                "trade": trade.model_dump() if trade else None,
            }
        )
    return rows_out

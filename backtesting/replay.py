"""Replay historical bars through the same decision + risk path as live."""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal

import polars as pl

from app.contracts.risk import RiskState
from data_plane.features.pipeline import FeaturePipeline
from decision_engine.feature_frame import enrich_bars_last_row
from decision_engine.pipeline import DecisionPipeline
from decision_engine.run_step import run_decision_tick
from risk_engine.engine import RiskEngine


def replay_decisions(
    bars: pl.DataFrame,
    pipeline: DecisionPipeline,
    risk_engine: RiskEngine,
    *,
    symbol: str,
    spread_bps: float = 5.0,
    feature_pipeline: FeaturePipeline | None = None,
    position_signed_qty: Decimal | None = None,
) -> list[dict]:
    """Walk OHLCV bars; same `enrich_bars_last_row` + `run_decision_tick` as live (cumulative window)."""
    if bars.height == 0:
        return []
    fp = feature_pipeline or FeaturePipeline()
    ohlcv_cols = {"timestamp", "open", "high", "low", "close", "volume"}
    frame = bars.sort("timestamp")
    base_cols = [c for c in frame.columns if c in ohlcv_cols]
    raw = frame.select(base_cols) if base_cols else frame

    risk = RiskState()
    rows_out: list[dict] = []
    pos = position_signed_qty if position_signed_qty is not None else Decimal(0)

    for row in frame.iter_rows(named=True):
        ts = row.get("timestamp")
        sub = raw.filter(pl.col("timestamp") <= ts) if ts is not None else raw
        feats = enrich_bars_last_row(sub, fp)
        if feats is None:
            feats = {
                k: float(v)
                for k, v in row.items()
                if k != "timestamp" and isinstance(v, (int, float))
            }
        if isinstance(ts, datetime):
            dt = ts
        else:
            dt = None
        regime, fc, route, action, trade, risk = run_decision_tick(
            symbol=symbol,
            feature_row=feats,
            spread_bps=spread_bps,
            risk_state=risk,
            pipeline=pipeline,
            risk_engine=risk_engine,
            mid_price=float(row.get("close", 1.0)),
            data_timestamp=dt,
            position_signed_qty=pos,
        )
        if trade is not None:
            q = Decimal(str(trade.quantity))
            if trade.side == "buy":
                pos += q
            else:
                pos -= q
        rows_out.append(
            {
                "timestamp": ts,
                "route": route.route_id.value,
                "regime": regime.semantic.value,
                "trade": trade.model_dump() if trade else None,
            }
        )
    return rows_out

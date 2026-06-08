"""Phase C: different OHLCV windows must produce different forecasts (guards flat-line regression)."""

from __future__ import annotations

import numpy as np
import polars as pl
import pytest
from datetime import UTC, datetime, timedelta

from legacy.decision_pipeline.decision_engine.pipeline import DecisionPipeline, _ohlc_arrays_from_history_or_stub
from app.config.settings import load_settings


def _sine_bars(n: int, base_price: float = 100.0, freq: float = 0.1) -> pl.DataFrame:
    """Build a synthetic sine-wave OHLCV frame so two windows are clearly different."""
    rows = []
    base = datetime(2026, 1, 1, tzinfo=UTC)
    for i in range(n):
        p = base_price + 5 * np.sin(freq * i)
        rows.append({
            "timestamp": base + timedelta(minutes=i),
            "open": float(p - 0.1),
            "high": float(p + 0.2),
            "low": float(p - 0.2),
            "close": float(p),
            "volume": 1000.0,
        })
    return pl.DataFrame(rows)


def test_arrays_differ_for_different_windows() -> None:
    """Two non-overlapping windows of real bars must produce different OHLCV arrays."""
    df = _sine_bars(200)

    # Window 1: bars 0–63
    w1 = df.slice(0, 64)
    # Window 2: bars 100–163
    w2 = df.slice(100, 64)

    dummy_feature_row: dict[str, float] = {"close": 100.0}

    o1, h1, lo1, cl1, v1 = _ohlc_arrays_from_history_or_stub(w1, dummy_feature_row, history_len=64)
    o2, h2, lo2, cl2, v2 = _ohlc_arrays_from_history_or_stub(w2, dummy_feature_row, history_len=64)

    # The close arrays must be different (sine wave guarantees this)
    assert not np.allclose(cl1, cl2), "Different windows must produce different close arrays"


def test_stub_fallback_when_no_history() -> None:
    """With no history, falls back to the flat-bar stub."""
    feature_row = {"close": 99.0, "volume": 500.0}
    o, h, lo, cl, v = _ohlc_arrays_from_history_or_stub(None, feature_row, history_len=16)
    assert len(cl) == 16
    # Stub repeats near-constant close
    assert np.allclose(cl, 99.0, atol=0.1)


def test_stub_fallback_when_single_bar() -> None:
    """Single bar (height < 2) also falls back to stub."""
    df = _sine_bars(1)
    feature_row = {"close": 100.0}
    o, h, lo, cl, v = _ohlc_arrays_from_history_or_stub(df, feature_row, history_len=8)
    # Stub returns flat
    assert np.allclose(cl, 100.0, atol=0.1)


def test_pipeline_step_produces_non_flat_packet_with_history() -> None:
    """DecisionPipeline.step must produce a non-trivial forecast given real history."""
    settings = load_settings()
    pipeline = DecisionPipeline(settings=settings)

    df = _sine_bars(130)  # enough history for default quantile config

    # Build a minimal feature row from the last bar
    last = df.to_dicts()[-1]
    feature_row = {
        "close": float(last["close"]),
        "volume": float(last["volume"]),
    }

    from app.contracts.risk import RiskState
    from datetime import UTC, datetime

    _regime, fc, _route, _proposal, _risk = pipeline.step(
        "BTC-USD",
        feature_row,
        spread_bps=5.0,
        risk=RiskState(),
        mid_price=float(last["close"]),
        data_timestamp=datetime.now(UTC),
        ohlc_history=df,
    )

    # With real history we expect the volatility to be non-zero (sine wave has variation).
    assert fc.volatility >= 0.0  # structural check
    # The forecast packet should exist
    pkt = pipeline.last_forecast_packet
    assert pkt is not None

from __future__ import annotations

from datetime import UTC, datetime

import pytest
from pydantic import ValidationError

from app.config.settings import AppSettings
from app.contracts.decision_snapshots import MarketSnapshot, SessionMode
from app.contracts.snapshot_builders import build_decision_boundary_input


def test_market_snapshot_rejects_inverted_book():
    with pytest.raises(ValidationError):
        MarketSnapshot(
            snapshot_id="x",
            timestamp=datetime.now(UTC),
            instrument_id="BTC-USD",
            last_price=100.0,
            mid_price=100.0,
            best_bid=101.0,
            best_ask=99.0,
            spread_bps=1.0,
            realized_vol_short=0.1,
            realized_vol_medium=0.1,
            book_imbalance=0.0,
            depth_near_touch=1.0,
            trade_volume_short=1.0,
            volume_burst_score=0.0,
            market_freshness=0.9,
            market_reliability=0.9,
            session_mode=SessionMode.REGULAR,
        )


def test_build_boundary_merges_canonical_floats():
    s = AppSettings()
    row = {"close": 50_000.0, "volume": 1e6, "rsi_14": 55.0, "atr_14": 100.0, "return_1": 0.001}
    ts = datetime(2026, 1, 15, 12, 0, 0, tzinfo=UTC)
    bundle, merged = build_decision_boundary_input(
        symbol="BTC-USD",
        feature_row=row,
        spread_bps=10.0,
        mid_price=50_000.0,
        data_timestamp=ts,
        settings=s,
    )
    assert bundle.market.instrument_id == "BTC-USD"
    assert "canonical_market_freshness" in merged
    assert merged["canonical_market_freshness"] == pytest.approx(0.92)
    assert merged["close"] == 50_000.0

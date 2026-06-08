"""FB-CAN-062: canonical contract conventions (UTC, confidence/freshness clipping)."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta, timezone

import pytest
from pydantic import ValidationError

from app.contracts.canonical_conventions import (
    clip_symmetric_unit,
    clip_unit_interval,
    ensure_utc_datetime,
    validate_decision_boundary_input_timestamps,
)
from app.contracts.decision_snapshots import (
    DecisionBoundaryInput,
    ExecutionFeedbackSnapshot,
    MarketSnapshot,
    OrderStyleUsed,
    SafetyRegimeSnapshot,
    ServiceConfigurationSnapshot,
    SessionMode,
    StructuralSignalSnapshot,
)


def test_ensure_utc_datetime_naive_is_utc():
    ts = datetime(2026, 1, 1, 12, 0, 0)
    u = ensure_utc_datetime(ts)
    assert u.tzinfo is UTC
    assert u.hour == 12


def test_clip_unit_interval_nan_and_overflow():
    assert clip_unit_interval(float("nan")) == 0.0
    assert clip_unit_interval(2.0) == 1.0
    assert clip_unit_interval(-0.5) == 0.0


def test_clip_symmetric_unit():
    assert clip_symmetric_unit(float("nan")) == 0.0
    assert clip_symmetric_unit(2.0) == 1.0
    assert clip_symmetric_unit(-1.5) == -1.0


def test_market_snapshot_clips_freshness_and_coerces_naive_timestamp():
    ts = datetime(2026, 6, 1, 0, 0, 0)
    m = MarketSnapshot(
        snapshot_id="s",
        timestamp=ts,
        instrument_id="BTC-USD",
        last_price=1.0,
        mid_price=1.0,
        best_bid=0.9,
        best_ask=1.1,
        spread_bps=1.0,
        realized_vol_short=0.1,
        realized_vol_medium=0.1,
        book_imbalance=0.0,
        depth_near_touch=1.0,
        trade_volume_short=1.0,
        volume_burst_score=0.0,
        market_freshness=99.0,
        market_reliability=-1.0,
        session_mode=SessionMode.REGULAR,
    )
    assert m.market_freshness == 1.0
    assert m.market_reliability == 0.0
    assert m.timestamp.tzinfo is UTC


def test_structural_clips_optional_scores():
    ts = datetime.now(UTC)
    s = StructuralSignalSnapshot(
        snapshot_id="s",
        timestamp=ts,
        instrument_id="X",
        gex_score=9.0,
        iv_skew_score=-9.0,
        stablecoin_flow_proxy=0.5,
        signal_freshness_structural=1.0,
        signal_reliability_structural=1.0,
    )
    assert s.gex_score == 1.0
    assert s.iv_skew_score == -1.0
    assert s.stablecoin_flow_proxy == 0.5


def test_boundary_timestamps_reject_non_datetime():
    with pytest.raises(ValidationError):
        MarketSnapshot(
            snapshot_id="s",
            timestamp="not-a-datetime",  # type: ignore[arg-type]
            instrument_id="BTC-USD",
            last_price=1.0,
            mid_price=1.0,
            best_bid=0.9,
            best_ask=1.1,
            spread_bps=1.0,
            realized_vol_short=0.1,
            realized_vol_medium=0.1,
            book_imbalance=0.0,
            depth_near_touch=1.0,
            trade_volume_short=1.0,
            volume_burst_score=0.0,
            market_freshness=0.5,
            market_reliability=0.5,
            session_mode=SessionMode.REGULAR,
        )


def test_validate_decision_boundary_input_timestamps_accepts_utc_bundle():
    ts = datetime(2026, 1, 1, tzinfo=timezone(timedelta(hours=2)))
    ts_utc = ts.astimezone(UTC)
    m = MarketSnapshot(
        snapshot_id="s",
        timestamp=ts_utc,
        instrument_id="BTC-USD",
        last_price=1.0,
        mid_price=1.0,
        best_bid=0.9,
        best_ask=1.1,
        spread_bps=1.0,
        realized_vol_short=0.1,
        realized_vol_medium=0.1,
        book_imbalance=0.0,
        depth_near_touch=1.0,
        trade_volume_short=1.0,
        volume_burst_score=0.0,
        market_freshness=0.5,
        market_reliability=0.5,
        session_mode=SessionMode.REGULAR,
    )
    st = StructuralSignalSnapshot(
        snapshot_id="s",
        timestamp=ts_utc,
        instrument_id="BTC-USD",
        signal_freshness_structural=0.5,
        signal_reliability_structural=0.5,
    )
    sa = SafetyRegimeSnapshot(
        snapshot_id="s",
        timestamp=ts_utc,
        instrument_id="BTC-USD",
        regime_probabilities={"a": 1.0},
        regime_confidence=0.5,
        transition_probability=0.5,
        novelty_score=0.5,
        crypto_heat_score=0.5,
        reflexivity_score=0.5,
        degradation_level="normal",
    )
    ex = ExecutionFeedbackSnapshot(
        feedback_id="f",
        timestamp=ts_utc,
        instrument_id="BTC-USD",
        expected_fill_price=1.0,
        realized_fill_price=1.0,
        realized_slippage_bps=0.0,
        fill_ratio=1.0,
        fill_latency_ms=0.0,
        execution_confidence_realized=0.5,
        venue_quality_score=0.5,
        order_style_used=OrderStyleUsed.MARKET,
    )
    svc = ServiceConfigurationSnapshot(snapshot_id="s", timestamp=ts_utc)
    inp = DecisionBoundaryInput(
        market=m,
        structural=st,
        safety=sa,
        execution_feedback=ex,
        service_config=svc,
    )
    validate_decision_boundary_input_timestamps(inp)


def test_validate_decision_boundary_input_timestamps_rejects_wrong_type():
    with pytest.raises(TypeError):
        validate_decision_boundary_input_timestamps(object())

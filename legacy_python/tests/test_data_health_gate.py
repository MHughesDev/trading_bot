"""Phase D: data-health gate blocks new entries on bad history; resumes on clean window."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from decimal import Decimal

import polars as pl
import pytest

from data_plane.health.data_health import DataHealthResult, check_data_health, RISK_BLOCK_DATA_HEALTH
from risk_engine.engine import RiskEngine, RISK_BLOCK_DATA_HEALTH as ENGINE_BLOCK
from app.config.settings import load_settings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState


def _make_bars(n: int, interval: int = 60, now: datetime | None = None) -> pl.DataFrame:
    """Build n contiguous completed bars ending at 'now'."""
    end = (now or datetime.now(UTC))
    rows = []
    for i in range(n - 1, -1, -1):
        ts = end - timedelta(seconds=interval * (i + 1))
        rows.append({
            "timestamp": ts,
            "open": 100.0,
            "high": 101.0,
            "low": 99.0,
            "close": 100.0,
            "volume": 1000.0,
        })
    return pl.DataFrame(rows).sort("timestamp")


# ------------------------------------------------------------------
# DataHealth unit tests
# ------------------------------------------------------------------

def test_healthy_window() -> None:
    now = datetime.now(UTC)
    bars = _make_bars(80, now=now)
    result = check_data_health("BTC-USD", bars, required_bars=60, interval_seconds=60, now=now)
    assert result.is_healthy
    assert not result.is_shallow
    assert not result.is_stale
    assert not result.has_interior_gap


def test_shallow_window_unhealthy() -> None:
    now = datetime.now(UTC)
    bars = _make_bars(10, now=now)
    result = check_data_health("BTC-USD", bars, required_bars=60, interval_seconds=60, now=now)
    assert not result.is_healthy
    assert result.is_shallow


def test_stale_window_unhealthy() -> None:
    old_now = datetime.now(UTC) - timedelta(hours=2)
    bars = _make_bars(80, interval=60, now=old_now)
    result = check_data_health(
        "BTC-USD", bars, required_bars=60, interval_seconds=60, max_staleness_seconds=300
    )
    assert not result.is_healthy
    assert result.is_stale


def test_gappy_window_unhealthy() -> None:
    now = datetime.now(UTC)
    rows = []
    # 30 contiguous, then a 10-minute gap, then 30 more
    for i in range(30):
        rows.append({
            "timestamp": now - timedelta(minutes=70 - i),
            "open": 100.0, "high": 101.0, "low": 99.0, "close": 100.0, "volume": 1000.0,
        })
    # Gap: skip 10 minutes
    for i in range(30):
        rows.append({
            "timestamp": now - timedelta(minutes=30 - i),
            "open": 100.0, "high": 101.0, "low": 99.0, "close": 100.0, "volume": 1000.0,
        })
    bars = pl.DataFrame(rows).sort("timestamp")
    result = check_data_health(
        "BTC-USD", bars, required_bars=50, interval_seconds=60, now=now
    )
    assert not result.is_healthy
    assert result.has_interior_gap


def test_none_bars_unhealthy() -> None:
    result = check_data_health("BTC-USD", None, required_bars=60, interval_seconds=60)
    assert not result.is_healthy
    assert result.is_shallow
    assert result.is_stale


# ------------------------------------------------------------------
# Risk engine integration
# ------------------------------------------------------------------

def test_risk_engine_blocks_on_data_integrity_alert() -> None:
    settings = load_settings()
    engine = RiskEngine(settings)
    risk = RiskState(data_integrity_alert=True)
    proposal = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )

    trade, new_risk = engine.evaluate(
        "BTC-USD",
        proposal,
        risk,
        mid_price=50000.0,
        spread_bps=5.0,
        data_timestamp=datetime.now(UTC),
        product_tradable=True,
    )

    assert trade is None
    assert ENGINE_BLOCK in (new_risk.last_risk_block_codes or [])


def test_risk_engine_allows_when_health_clean() -> None:
    settings = load_settings()
    engine = RiskEngine(settings)
    # data_integrity_alert = False (or None)
    risk = RiskState(data_integrity_alert=False)
    proposal = ActionProposal(
        symbol="BTC-USD",
        route_id=RouteId.INTRADAY,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )

    trade, new_risk = engine.evaluate(
        "BTC-USD",
        proposal,
        risk,
        mid_price=50000.0,
        spread_bps=5.0,
        data_timestamp=datetime.now(UTC),
        product_tradable=True,
    )

    # Should NOT be blocked by the data health code (may still be blocked by other rules)
    assert ENGINE_BLOCK not in (new_risk.last_risk_block_codes or [])


def test_data_health_block_code_consistency() -> None:
    """The constant in data_health.py and engine.py must be the same string."""
    assert RISK_BLOCK_DATA_HEALTH == ENGINE_BLOCK == "risk_data_health"

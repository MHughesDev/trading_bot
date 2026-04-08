"""Replay portfolio solvency when track_portfolio=True (Issue 33)."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal
from unittest.mock import MagicMock

import polars as pl

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteDecision, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from backtesting.execution_params import BacktestExecutionParams
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_replay_skips_buy_when_insolvent(monkeypatch) -> None:
    t = datetime(2025, 1, 1, 0, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [{"timestamp": t, "open": 100.0, "high": 100.0, "low": 100.0, "close": 100.0, "volume": 1.0}]
    )

    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[1.0, 0, 0, 0],
        confidence=0.9,
    )
    fc = ForecastOutput(
        returns_1=0.01,
        returns_3=0.01,
        returns_5=0.01,
        returns_15=0.01,
        volatility=0.02,
        uncertainty=0.1,
    )
    from app.contracts.decisions import RouteId as R

    route = RouteDecision(route_id=R.SCALPING, confidence=0.8)
    proposal = ActionProposal(
        symbol="BTC-USD",
        route_id=R.SCALPING,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )
    trade = TradeAction(
        symbol="BTC-USD",
        side="buy",
        quantity=Decimal("10"),
        order_type="market",
        limit_price=None,
        stop_price=None,
        time_in_force="gtc",
        route_id=R.SCALPING,
    )

    def fake_tick(**_kwargs):
        return regime, fc, route, proposal, trade, RiskState()

    monkeypatch.setattr("backtesting.replay.run_decision_tick", fake_tick)

    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=1.0,
        track_portfolio=True,
        execution_params=BacktestExecutionParams(
            slippage_bps=0.0,
            fee_bps=0.0,
            slippage_noise_bps=0.0,
            rng_seed=1,
            initial_cash=Decimal("50"),
            enforce_solvency=True,
        ),
    )
    assert len(out) == 1
    row = out[0]
    assert row["solvency_blocked"] is True
    assert row["trade"] is None
    assert row["portfolio_cash"] == "50"


def test_replay_allows_buy_when_solvent(monkeypatch) -> None:
    t = datetime(2025, 1, 1, 0, 0, 0, tzinfo=UTC)
    df = pl.DataFrame(
        [{"timestamp": t, "open": 100.0, "high": 100.0, "low": 100.0, "close": 100.0, "volume": 1.0}]
    )

    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.BULL,
        probabilities=[1.0, 0, 0, 0],
        confidence=0.9,
    )
    fc = ForecastOutput(
        returns_1=0.01,
        returns_3=0.01,
        returns_5=0.01,
        returns_15=0.01,
        volatility=0.02,
        uncertainty=0.1,
    )
    from app.contracts.decisions import RouteId as R

    route = RouteDecision(route_id=R.SCALPING, confidence=0.8)
    proposal = ActionProposal(
        symbol="BTC-USD",
        route_id=R.SCALPING,
        direction=1,
        size_fraction=0.5,
        stop_distance_pct=0.01,
    )
    trade = TradeAction(
        symbol="BTC-USD",
        side="buy",
        quantity=Decimal("0.1"),
        order_type="market",
        limit_price=None,
        stop_price=None,
        time_in_force="gtc",
        route_id=R.SCALPING,
    )

    monkeypatch.setattr(
        "backtesting.replay.run_decision_tick",
        lambda **_kwargs: (regime, fc, route, proposal, trade, RiskState()),
    )

    pipe = MagicMock(spec=DecisionPipeline)
    eng = RiskEngine(AppSettings())
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        track_portfolio=True,
        execution_params=BacktestExecutionParams(
            slippage_bps=0.0,
            fee_bps=10.0,
            initial_cash=Decimal("100000"),
            enforce_solvency=True,
        ),
    )
    assert out[0]["trade"] is not None
    assert out[0]["solvency_blocked"] is False

"""Multi-symbol portfolio replay (Issue 32)."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal

import polars as pl

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteDecision, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from backtesting.execution_params import BacktestExecutionParams
from backtesting.replay import replay_multi_asset_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _stub_tick():
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
        symbol="X",
        route_id=R.SCALPING,
        direction=1,
        size_fraction=0.01,
        stop_distance_pct=0.01,
    )

    def make_trade(sym: str, qty: str):
        return TradeAction(
            symbol=sym,
            side="buy",
            quantity=Decimal(qty),
            order_type="market",
            limit_price=None,
            stop_price=None,
            time_in_force="gtc",
            route_id=R.SCALPING,
        )

    return regime, fc, route, proposal, make_trade


def test_replay_multi_shared_portfolio_two_symbols(monkeypatch) -> None:
    t = datetime(2025, 1, 1, 0, 0, 0, tzinfo=UTC)
    base = {"timestamp": t, "open": 100.0, "high": 100.0, "low": 100.0, "close": 100.0, "volume": 1.0}
    df_a = pl.DataFrame([base])
    df_b = pl.DataFrame([{**base, "close": 50.0}])

    regime, fc, route, proposal, make_trade = _stub_tick()
    calls: list[str] = []

    def fake_tick(*, symbol, **_kwargs):
        calls.append(symbol)
        tr = make_trade(symbol, "1" if symbol == "AAA-USD" else "2")
        return regime, fc, route, proposal, tr, RiskState()

    monkeypatch.setattr("backtesting.replay.run_decision_tick", fake_tick)

    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    out = replay_multi_asset_decisions(
        {"AAA-USD": df_a, "BBB-USD": df_b},
        pipe,
        eng,
        track_portfolio=True,
        execution_params=BacktestExecutionParams(
            slippage_bps=0.0,
            fee_bps=0.0,
            initial_cash=Decimal("10000"),
            enforce_solvency=True,
        ),
    )
    assert len(out) == 1
    row = out[0]
    assert "AAA-USD" in row["symbols"]
    assert "BBB-USD" in row["symbols"]
    assert row["portfolio_cash"] != "10000"  # spent on two buys
    assert "AAA-USD" in calls and "BBB-USD" in calls
    # Deterministic symbol order
    assert calls == ["AAA-USD", "BBB-USD"]


def test_replay_multi_staggered_timestamps(monkeypatch) -> None:
    t0 = datetime(2025, 1, 1, 0, 0, 0, tzinfo=UTC)
    t1 = datetime(2025, 1, 1, 0, 1, 0, tzinfo=UTC)
    df_a = pl.DataFrame(
        [
            {"timestamp": t0, "open": 10.0, "high": 10.0, "low": 10.0, "close": 10.0, "volume": 1.0},
            {"timestamp": t1, "open": 10.0, "high": 10.0, "low": 10.0, "close": 10.0, "volume": 1.0},
        ]
    )
    df_b = pl.DataFrame(
        [{"timestamp": t1, "open": 20.0, "high": 20.0, "low": 20.0, "close": 20.0, "volume": 1.0}]
    )

    regime, fc, route, proposal, make_trade = _stub_tick()

    def fake_tick(*, symbol, **_kwargs):
        tr = make_trade(symbol, "1")
        return regime, fc, route, proposal, tr, RiskState()

    monkeypatch.setattr("backtesting.replay.run_decision_tick", fake_tick)

    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    out = replay_multi_asset_decisions(
        {"A": df_a, "B": df_b},
        pipe,
        eng,
        track_portfolio=False,
    )
    assert len(out) == 2
    assert out[0]["timestamp"] == t0
    assert "A" in out[0]["symbols"] and "B" not in out[0]["symbols"]
    assert "A" in out[1]["symbols"] and "B" in out[1]["symbols"]

from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings
from backtesting.replay import replay_decisions
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def test_replay_uses_enriched_features():
    rows = []
    for i in range(30):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append({"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0})
    df = pl.DataFrame(rows)
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    out = replay_decisions(df, pipe, eng, symbol="BTC-USD", spread_bps=5.0)
    assert len(out) == 30
    assert "route" in out[-1]

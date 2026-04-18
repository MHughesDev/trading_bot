"""FB-CAN-068: deterministic replay seed policy and replay_provenance on outputs."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl
import pytest

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayRunContract
from backtesting.replay import replay_decisions
from backtesting.replay_provenance import (
    contract_identity_hash,
    dataset_fingerprint,
    deterministic_seed_from_dataset,
    resolve_replay_seed,
)
from decision_engine.pipeline import DecisionPipeline
from risk_engine.engine import RiskEngine


def _small_bars() -> pl.DataFrame:
    rows = []
    for i in range(4):
        t = datetime(2025, 1, 1, 0, i, 0, tzinfo=UTC)
        p = 100.0 + i * 0.1
        rows.append(
            {"timestamp": t, "open": p, "high": p + 0.05, "low": p - 0.05, "close": p, "volume": 1.0}
        )
    return pl.DataFrame(rows)


def test_resolve_replay_seed_contract_over_exec_params() -> None:
    fp = "a" * 64
    c = ReplayRunContract(
        replay_run_id="r1",
        dataset_id="d1",
        instrument_scope=["X"],
        seed=99,
    )
    from backtesting.execution_params import BacktestExecutionParams

    ep = BacktestExecutionParams(
        slippage_bps=1.0,
        fee_bps=1.0,
        rng_seed=42,
    )
    s, deriv = resolve_replay_seed(c, exec_params=ep, dataset_fp=fp)
    assert s == 99
    assert deriv == "contract"


def test_deterministic_seed_stable_for_same_inputs() -> None:
    df = _small_bars()
    dfp = dataset_fingerprint(df)
    c = ReplayRunContract(
        replay_run_id="rid",
        dataset_id="did",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["BTC-USD"],
    )
    s1 = deterministic_seed_from_dataset(dfp, c)
    s2 = deterministic_seed_from_dataset(dfp, c)
    assert s1 == s2
    c2 = c.model_copy(update={"replay_run_id": "other"})
    assert deterministic_seed_from_dataset(dfp, c2) != s1


def test_replay_rows_include_matching_provenance_hashes() -> None:
    df = _small_bars()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="prov-test",
        dataset_id="unit",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["BTC-USD"],
    )
    a = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=5.0,
        replay_contract=contract,
    )
    b = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=5.0,
        replay_contract=contract,
    )
    assert len(a) == len(b) >= 1
    pa = a[0]["replay_provenance"]
    pb = b[0]["replay_provenance"]
    assert isinstance(pa, dict)
    assert pa["replay_run_id"] == "prov-test"
    assert pa["contract_identity_hash"] == contract_identity_hash(contract)
    assert pa["reproducibility_hash"] == pb["reproducibility_hash"]
    assert pa["seed_derivation"] == "dataset_fingerprint"
    assert pa["seed_effective"] == deterministic_seed_from_dataset(
        dataset_fingerprint(df.sort("timestamp")),
        contract,
    )


@pytest.mark.parametrize("track_portfolio", [False, True])
def test_replay_provenance_present_with_track_portfolio(track_portfolio: bool) -> None:
    df = _small_bars()
    pipe = DecisionPipeline()
    eng = RiskEngine(AppSettings())
    contract = ReplayRunContract(
        replay_run_id="tp-test",
        dataset_id="unit",
        instrument_scope=["BTC-USD"],
    )
    out = replay_decisions(
        df,
        pipe,
        eng,
        symbol="BTC-USD",
        spread_bps=5.0,
        replay_contract=contract,
        track_portfolio=track_portfolio,
    )
    assert all("replay_provenance" in r for r in out)

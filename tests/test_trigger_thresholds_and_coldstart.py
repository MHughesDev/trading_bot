"""Injectable trigger thresholds and cold-start synthetic-forecast surfacing/gating."""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl

from app.config.settings import AppSettings, load_settings
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline, _is_cold_start
from decision_engine.trigger_engine import TriggerThresholds, evaluate_trigger


def _apex() -> CanonicalStateOutput:
    return CanonicalStateOutput(
        regime_probabilities=[0.25, 0.25, 0.2, 0.15, 0.15],
        regime_confidence=0.4,
        transition_probability=0.2,
        novelty=0.1,
        heat_score=0.2,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 3, 5],
        q_low=[-0.02, -0.03, -0.04],
        q_med=[0.01, 0.0, 0.0],
        q_high=[0.04, 0.05, 0.06],
        interval_width=[0.06, 0.08, 0.1],
        regime_vector=[0.3, 0.3, 0.2, 0.2],
        confidence_score=0.75,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.05,
    )


def test_trigger_thresholds_from_settings() -> None:
    s = AppSettings(
        trigger_setup_threshold=0.5,
        trigger_pretrigger_threshold=0.4,
        trigger_confirm_threshold=0.6,
    )
    thr = TriggerThresholds.from_settings(s)
    assert thr.setup_threshold == 0.5
    assert thr.pretrigger_threshold == 0.4
    assert thr.confirm_threshold == 0.6
    # Defaults preserved for un-exposed fields.
    assert thr.entry_extension_limit == 0.85


def test_trigger_threshold_gates_setup() -> None:
    feats = {"close": 50_000.0, "rsi_14": 55.0, "return_1": 0.001, "volume": 1e6}
    permissive = evaluate_trigger(
        _pkt(), feats, spread_bps=5.0, apex=_apex(),
        thresholds=TriggerThresholds(setup_threshold=0.0, setup_exec_floor=0.0),
    )
    strict = evaluate_trigger(
        _pkt(), feats, spread_bps=5.0, apex=_apex(),
        thresholds=TriggerThresholds(setup_threshold=0.99),
    )
    assert permissive.setup_valid is True
    assert strict.setup_valid is False


def test_default_behavior_unchanged_without_thresholds() -> None:
    # Omitting thresholds uses the historical defaults (0.22/0.18/0.2): scores are in range.
    feats = {"close": 50_000.0, "rsi_14": 55.0, "return_1": 0.001, "volume": 1e6}
    out = evaluate_trigger(_pkt(), feats, spread_bps=5.0, apex=_apex())
    assert 0.0 <= out.setup_score <= 1.0


def _sine_bars(n: int) -> pl.DataFrame:
    import numpy as np

    t = np.arange(n)
    close = 100.0 + 2.0 * np.sin(t / 6.0)
    return pl.DataFrame(
        {
            "open": close,
            "high": close + 0.1,
            "low": close - 0.1,
            "close": close,
            "volume": np.full(n, 1e6),
        }
    )


def test_is_cold_start_predicate() -> None:
    assert _is_cold_start(None) is True
    assert _is_cold_start(_sine_bars(1)) is True
    assert _is_cold_start(_sine_bars(2)) is False


def test_pipeline_marks_cold_start_in_diagnostics() -> None:
    pipeline = DecisionPipeline(settings=load_settings())
    pipeline.step(
        "BTC-USD",
        {"close": 100.0, "volume": 1e6},
        spread_bps=5.0,
        risk=RiskState(),
        mid_price=100.0,
        data_timestamp=datetime.now(UTC),
        ohlc_history=None,
    )
    pkt = pipeline.last_forecast_packet
    assert pkt is not None
    assert pkt.forecast_diagnostics["cold_start_synthetic"] is True

    df = _sine_bars(130)
    last = df.to_dicts()[-1]
    pipeline.step(
        "BTC-USD",
        {"close": float(last["close"]), "volume": float(last["volume"])},
        spread_bps=5.0,
        risk=RiskState(),
        mid_price=float(last["close"]),
        data_timestamp=datetime.now(UTC),
        ohlc_history=df,
    )
    assert pipeline.last_forecast_packet.forecast_diagnostics["cold_start_synthetic"] is False

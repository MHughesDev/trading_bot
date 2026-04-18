"""FB-CAN-077 immutable run binding on decision records."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.config.settings import AppSettings
from app.contracts.decisions import RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.replay_events import ReplayRunContract
from app.contracts.risk import RiskState
from app.contracts.run_binding import (
    build_run_binding,
    compute_run_binding_hash,
    effective_seed_live,
    resolve_effective_seed,
)
from decision_engine.decision_record import build_decision_record


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=[1, 3, 5],
        q_low=[-0.01, -0.02, -0.03],
        q_med=[0.0, 0.0, 0.0],
        q_high=[0.01, 0.02, 0.03],
        interval_width=[0.02, 0.04, 0.06],
        regime_vector=[0.4, 0.3, 0.2, 0.1],
        confidence_score=0.8,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.1,
    )


def test_build_run_binding_tamper_evident() -> None:
    c = ReplayRunContract(
        replay_run_id="r1",
        dataset_id="ds",
        config_version="2.0.0",
        logic_version="3.0.0",
        instrument_scope=["BTC-USD"],
        seed=42,
    )
    rb = build_run_binding(
        config_version="2.0.0",
        logic_version="3.0.0",
        dataset_id="ds",
        seed_effective=42,
        replay_contract=c,
    )
    assert rb.seed_effective == 42
    assert rb.contract_identity_hash
    assert rb.replay_run_id == "r1"
    h1 = rb.run_binding_hash
    h2 = compute_run_binding_hash(
        config_version="2.0.0",
        logic_version="3.0.0",
        dataset_id="ds",
        seed_effective=42,
        contract_identity_hash=rb.contract_identity_hash,
        replay_run_id="r1",
    )
    assert h1 == h2
    h_bad = compute_run_binding_hash(
        config_version="2.0.1",
        logic_version="3.0.0",
        dataset_id="ds",
        seed_effective=42,
        contract_identity_hash=rb.contract_identity_hash,
        replay_run_id="r1",
    )
    assert h_bad != h1


def test_resolve_effective_seed_order() -> None:
    c = ReplayRunContract(
        replay_run_id="r",
        dataset_id="d",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["X"],
        seed=99,
    )
    assert resolve_effective_seed(
        replay_contract=c,
        replay_dataset_fingerprint="abc",
        config_version="1.0.0",
        logic_version="1.0.0",
        live_dataset_id="live",
    ) == 99
    c2 = c.model_copy(update={"seed": None})
    s_fp = resolve_effective_seed(
        replay_contract=c2,
        replay_dataset_fingerprint="deadbeef" * 8,
        config_version="1.0.0",
        logic_version="1.0.0",
        live_dataset_id="live",
    )
    assert isinstance(s_fp, int)
    s_live = resolve_effective_seed(
        replay_contract=c2,
        replay_dataset_fingerprint=None,
        config_version="1.0.0",
        logic_version="1.0.0",
        live_dataset_id="live",
    )
    assert s_live == effective_seed_live(
        config_version="1.0.0", logic_version="1.0.0", dataset_id="d"
    )


def test_decision_record_has_run_binding_live() -> None:
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.SIDEWAYS,
        probabilities=[1.0, 0.0, 0.0, 0.0],
        confidence=0.0,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.0,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[RouteId.NO_TRADE])
    risk = RiskState()
    dr = build_decision_record(
        symbol="BTC-USD",
        data_timestamp=datetime.now(UTC),
        settings=AppSettings(),
        regime=regime,
        forecast=fc,
        route=route,
        proposal=None,
        risk=risk,
        forecast_packet=_pkt(),
        trade=None,
    )
    assert dr.run_binding is not None
    assert dr.run_binding.dataset_id == "live"
    assert dr.run_binding.config_version
    assert dr.run_binding.logic_version
    assert len(dr.run_binding.run_binding_hash) == 64


def test_decision_record_replay_uses_dataset_and_contract() -> None:
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.SIDEWAYS,
        probabilities=[1.0, 0.0, 0.0, 0.0],
        confidence=0.0,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.0,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[RouteId.NO_TRADE])
    risk = RiskState()
    contract = ReplayRunContract(
        replay_run_id="unit-replay",
        dataset_id="parquet-2024q1",
        config_version="1.0.0",
        logic_version="1.0.0",
        instrument_scope=["BTC-USD"],
    )
    dr = build_decision_record(
        symbol="BTC-USD",
        data_timestamp=datetime.now(UTC),
        settings=AppSettings(),
        regime=regime,
        forecast=fc,
        route=route,
        proposal=None,
        risk=risk,
        forecast_packet=_pkt(),
        trade=None,
        replay_contract=contract,
        replay_dataset_fingerprint="a" * 64,
    )
    rb = dr.run_binding
    assert rb is not None
    assert rb.dataset_id == "parquet-2024q1"
    assert rb.replay_run_id == "unit-replay"
    assert rb.contract_identity_hash


def test_strict_run_binding_rejects_fp_without_contract() -> None:
    s = AppSettings()
    dom = s.canonical.domains
    if hasattr(dom, "replay"):
        dom.replay["strict_run_binding"] = True
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.SIDEWAYS,
        probabilities=[1.0, 0.0, 0.0, 0.0],
        confidence=0.0,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.0,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[RouteId.NO_TRADE])
    with pytest.raises(ValueError, match="strict_run_binding"):
        build_decision_record(
            symbol="BTC-USD",
            data_timestamp=datetime.now(UTC),
            settings=s,
            regime=regime,
            forecast=fc,
            route=route,
            proposal=None,
            risk=RiskState(),
            forecast_packet=_pkt(),
            trade=None,
            replay_contract=None,
            replay_dataset_fingerprint="abc",
        )

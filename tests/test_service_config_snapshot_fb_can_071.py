"""FB-CAN-071: ServiceConfigurationSnapshot bindings + replay threading."""

from __future__ import annotations

from datetime import UTC, datetime

from app.config.settings import AppSettings
from app.contracts.replay_events import ReplayMode, ReplayRunContract
from app.contracts.snapshot_builders import build_decision_boundary_input
from app.contracts.risk import RiskState
from decision_engine.pipeline import DecisionPipeline


def _features(close: float = 50_000.0) -> dict[str, float]:
    feats = {f"f{i}": float(i) * 0.01 for i in range(32)}
    feats["close"] = close
    feats["volume"] = 1e6
    return feats


def test_boundary_input_replay_contract_overrides_versions_and_scope() -> None:
    s = AppSettings()
    row = {"close": 50_000.0, "volume": 1e6, "rsi_14": 55.0, "atr_14": 100.0, "return_1": 0.001}
    ts = datetime(2026, 1, 15, 12, 0, 0, tzinfo=UTC)
    c = ReplayRunContract(
        replay_run_id="run-abc",
        dataset_id="ds-xyz",
        config_version="9.9.9",
        logic_version="logic-replay",
        replay_mode=ReplayMode.HISTORICAL_NOMINAL,
    )
    bundle, _merged = build_decision_boundary_input(
        symbol="BTC-USD",
        feature_row=row,
        spread_bps=10.0,
        mid_price=50_000.0,
        data_timestamp=ts,
        settings=s,
        replay_contract=c,
    )
    svc = bundle.service_config
    assert svc.config_version == "9.9.9"
    assert svc.logic_version == "logic-replay"
    assert svc.environment_scope == "simulation"
    assert svc.extra.get("replay_run_id") == "run-abc"
    assert svc.extra.get("replay_dataset_id") == "ds-xyz"


def test_pipeline_step_passes_replay_contract_to_service_config_diagnostics() -> None:
    pipe = DecisionPipeline(settings=AppSettings())
    risk = RiskState()
    ts = datetime(2026, 2, 1, 15, 30, 0, tzinfo=UTC)
    c = ReplayRunContract(
        replay_run_id="pipe-run",
        dataset_id="pipe-ds",
        config_version="3.1.0",
        logic_version="lv-pipe",
        replay_mode=ReplayMode.SHADOW_COMPARISON,
    )
    pipe.step(
        "BTC-USD",
        _features(),
        spread_bps=5.0,
        risk=risk,
        mid_price=50_000.0,
        portfolio_equity_usd=100_000.0,
        data_timestamp=ts,
        replay_contract=c,
    )
    pkt = pipe.last_forecast_packet
    assert pkt is not None
    cbi = pkt.forecast_diagnostics.get("canonical_boundary_input")
    assert isinstance(cbi, dict)
    sc = cbi.get("service_config")
    assert isinstance(sc, dict)
    assert sc.get("config_version") == "3.1.0"
    assert sc.get("logic_version") == "lv-pipe"
    assert sc.get("environment_scope") == "shadow"
    extra = sc.get("extra") or {}
    assert extra.get("replay_run_id") == "pipe-run"

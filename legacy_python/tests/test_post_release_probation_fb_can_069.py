"""FB-CAN-069: post-release live probation policy and evaluation."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from app.config.post_release_probation import probation_policy_from_settings
from app.config.settings import AppSettings
from app.contracts.release_objects import ReleaseCandidate, ReleaseLedger, write_release_ledger
from observability.drift_calibration_metrics import (
    get_probation_rolling_samples,
    percentile_95,
    record_calibration_and_drift_from_tick,
    reset_probation_sample_buffers,
)
from orchestration.post_release_probation import evaluate_post_release_probation_tick


def test_probation_policy_from_default_yaml_settings() -> None:
    s = AppSettings()
    p = probation_policy_from_settings(s)
    assert p.enabled is True
    assert p.windows.active_hours == 48.0


def test_percentile_95() -> None:
    assert percentile_95(list(range(100))) == pytest.approx(94.0)


def _append_trade_intent_tick(
    *,
    erosion: float,
    drift_penalty: float = 0.1,
    fp_mem: float = 0.1,
) -> None:
    class _R:
        trigger_false_positive_memory = fp_mem
        last_decision_record = {
            "outcome": "trade_intent",
            "forecast_summary": {},
            "trade_intent": {
                "decision_confidence": 0.9,
                "trigger_confidence": 0.9,
                "execution_confidence": max(0.0, 0.81 - erosion),
            },
        }

    record_calibration_and_drift_from_tick(
        symbol="BTC-USD",
        risk=_R(),
        forecast_packet=None,
        feature_row={"canonical_exec_quality_penalty": drift_penalty},
        record_probation_samples=True,
    )


def test_evaluate_probation_abort_on_edge_erosion(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    approved = datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC)
    cand = ReleaseCandidate(
        release_id="rel-prob-1",
        kind="combined",
        owner="t",
        config_version="1.0.0",
        logic_version="1.0.0",
        environment="live",
        current_stage="active_live",
        approved_at=approved.isoformat(),
    )
    led = ReleaseLedger(candidates=[cand])
    path = tmp_path / "rl.json"
    write_release_ledger(led, path=path)

    s = AppSettings()
    monkeypatch.setattr(s.canonical.metadata, "logic_version", "1.0.0")

    reset_probation_sample_buffers("rel-prob-1")
    for _ in range(40):
        _append_trade_intent_tick(erosion=0.5)

    out = evaluate_post_release_probation_tick(
        settings=s,
        risk_engine=None,
        now=approved + timedelta(hours=1),
        ledger_path=path,
    )
    assert out.get("abort_recommended") is True
    assert out.get("phase") == "active"
    reasons = out.get("reasons") or []
    assert any("edge_erosion_p95" in r for r in reasons)


def test_prepare_buffers_clears_on_release_change(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    import orchestration.post_release_probation as prp_mod

    from orchestration.post_release_probation import prepare_probation_sample_buffers_before_metrics

    prp_mod._LAST_PREPARED_RELEASE = None
    reset_probation_sample_buffers("test-init-z")
    reset_probation_sample_buffers("rel-a")

    approved = datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC)
    c1 = ReleaseCandidate(
        release_id="rel-a",
        kind="combined",
        owner="t",
        config_version="1.0.0",
        logic_version="1.0.0",
        environment="live",
        current_stage="active_live",
        approved_at=approved.isoformat(),
    )
    write_release_ledger(ReleaseLedger(candidates=[c1]), path=tmp_path / "a.json")
    s = AppSettings()
    monkeypatch.setattr(s.canonical.metadata, "logic_version", "1.0.0")

    _append_trade_intent_tick(erosion=0.2)
    e1, _, _ = get_probation_rolling_samples(500)
    assert len(e1) == 1

    prepare_probation_sample_buffers_before_metrics(settings=s, ledger_path=tmp_path / "a.json")
    e2, _, _ = get_probation_rolling_samples(500)
    assert len(e2) == 1

    c2 = ReleaseCandidate(
        release_id="rel-b",
        kind="combined",
        owner="t",
        config_version="1.0.0",
        logic_version="1.0.0",
        environment="live",
        current_stage="active_live",
        approved_at=(approved + timedelta(seconds=1)).isoformat(),
    )
    write_release_ledger(ReleaseLedger(candidates=[c2]), path=tmp_path / "b.json")
    prepare_probation_sample_buffers_before_metrics(settings=s, ledger_path=tmp_path / "b.json")
    e3, _, _ = get_probation_rolling_samples(500)
    assert len(e3) == 0

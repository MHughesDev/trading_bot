from __future__ import annotations

import pytest

from models.registry.experiment_registry import (
    ExperimentRecord,
    ExperimentRegistry,
    link_experiment_to_release,
    query_experiments,
    read_experiment_registry,
    suggest_experiment_id,
    upsert_experiment,
    validate_experiment_transition,
    write_experiment_registry,
)
from orchestration.release_gating import ReleaseCandidate, ReleaseLedger, write_release_ledger


def test_suggest_experiment_id_stable():
    a = suggest_experiment_id("Trigger threshold sweep", "alice")
    b = suggest_experiment_id("Trigger threshold sweep", "alice")
    assert a == b
    assert a.startswith("exp-")


def test_upsert_and_query(tmp_path):
    path = tmp_path / "reg.json"
    reg = ExperimentRegistry()
    e1 = ExperimentRecord(
        experiment_id="exp-1",
        title="t1",
        owner="alice",
        domain="trigger_research",
        status="rejected",
        hypothesis="threshold X reduces noise",
        metrics_defined_before_run=["false_positive_rate"],
        change_type="new_trigger_rule",
        affected_components=["decision_engine/trigger_engine.py"],
        notes="over-throttling in stress",
        tags=["trigger"],
    )
    reg = upsert_experiment(reg, e1)
    write_experiment_registry(reg, path)
    loaded = read_experiment_registry(path)
    assert loaded is not None
    trig = query_experiments(loaded, domain="trigger_research")
    assert len(trig) == 1
    rej = query_experiments(loaded, status="rejected")
    assert len(rej) == 1
    notes = query_experiments(loaded, notes_substring="throttl")
    assert len(notes) == 1


def test_link_to_release(tmp_path, monkeypatch: pytest.MonkeyPatch):
    path = tmp_path / "reg.json"
    ledger_path = tmp_path / "ledger.json"
    write_release_ledger(
        ReleaseLedger(
            candidates=[
                ReleaseCandidate(
                    release_id="rel-1",
                    kind="config",
                    owner="ops",
                    config_version="1.0.0",
                )
            ]
        ),
        ledger_path,
    )
    e = ExperimentRecord(experiment_id="e99", title="x")
    import models.registry.experiment_registry as er_mod

    monkeypatch.setattr(er_mod, "default_release_ledger_path", lambda: ledger_path)

    reg = upsert_experiment(ExperimentRegistry(), e, ledger_path=ledger_path)
    reg2 = link_experiment_to_release(
        reg, experiment_id="e99", release_candidate_id="rel-1", ledger_path=ledger_path
    )
    write_experiment_registry(reg2, path)
    linked = query_experiments(read_experiment_registry(path) or ExperimentRegistry(), linked_release="rel-1")
    assert len(linked) == 1
    led_back = __import__("orchestration.release_gating", fromlist=["read_release_ledger"]).read_release_ledger(
        ledger_path
    )
    assert led_back is not None
    assert led_back.candidates[0].linked_experiment_ids == ["e99"]


def test_transition_validation():
    validate_experiment_transition("draft", "running")
    with pytest.raises(ValueError):
        validate_experiment_transition("draft", "candidate_for_release")

from __future__ import annotations

from models.registry.experiment_registry import (
    ExperimentRecord,
    ExperimentRegistry,
    link_experiment_to_release,
    query_experiments,
    read_experiment_registry,
    suggest_experiment_id,
    upsert_experiment,
    write_experiment_registry,
)


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
        domain="trigger_research",
        status="rejected",
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


def test_link_to_release(tmp_path):
    path = tmp_path / "reg.json"
    e = ExperimentRecord(experiment_id="e99", title="x")
    reg = upsert_experiment(ExperimentRegistry(), e)
    reg2 = link_experiment_to_release(reg, experiment_id="e99", release_candidate_id="rel-1")
    write_experiment_registry(reg2, path)
    linked = query_experiments(read_experiment_registry(path) or ExperimentRegistry(), linked_release="rel-1")
    assert len(linked) == 1

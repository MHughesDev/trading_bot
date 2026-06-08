"""FB-SPEC-06: filesystem active model set."""

from __future__ import annotations

from pathlib import Path

from models.registry.store import merge_registry_into_serving_view, read_active_model_set, write_active_model_set


def test_read_missing_returns_none(tmp_path: Path) -> None:
    assert read_active_model_set(tmp_path / "nope.json") is None


def test_merge_detects_drift(tmp_path: Path) -> None:
    reg = tmp_path / "r.json"
    write_active_model_set(
        reg,
        {
            "forecaster_weights_npz_path": "/a/f.npz",
            "forecaster_checkpoint_id": "v1",
        },
    )
    loaded = read_active_model_set(reg)
    assert loaded is not None
    m = merge_registry_into_serving_view(
        registry=loaded,
        env_forecaster_weights="/b/f.npz",
        env_policy_mlp=None,
        env_conformal=None,
        env_lineage_id="v1",
    )
    assert m["registry_loaded"] is True
    assert m["aligned_with_env"] is False
    assert any("forecaster_weights_npz_path" in x for x in m["drift_vs_env"])


def test_merge_aligned_when_matching(tmp_path: Path) -> None:
    reg = tmp_path / "r.json"
    p = str(tmp_path / "w.npz")
    write_active_model_set(
        reg,
        {
            "forecaster_weights_npz_path": p,
            "forecaster_checkpoint_id": "cid",
        },
    )
    loaded = read_active_model_set(reg)
    m = merge_registry_into_serving_view(
        registry=loaded,
        env_forecaster_weights=p,
        env_policy_mlp=None,
        env_conformal=None,
        env_lineage_id="cid",
    )
    assert m["aligned_with_env"] is True

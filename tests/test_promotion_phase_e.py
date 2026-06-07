"""Phase E / P1: promotion effect atomically updates the active-set manifest."""

from __future__ import annotations

import json
from pathlib import Path

from training_pipeline.orchestration.promotion import (
    PromotionDecision,
    apply_promotion_effect,
)
from datetime import UTC, datetime


def _make_decision(decision: str = "promote") -> PromotionDecision:
    return PromotionDecision(
        component="forecaster",
        current_champion_id=None,
        candidate_id="run-001",
        decision=decision,  # type: ignore[arg-type]
        reasons=["test"],
        comparison_metrics={},
        timestamp=datetime.now(UTC).isoformat(),
    )


def test_promote_creates_active_set_manifest(tmp_path) -> None:
    artifact = tmp_path / "run-001" / "forecaster_quantile_real.joblib"
    artifact.parent.mkdir(parents=True)
    artifact.write_bytes(b"fake")

    asp = tmp_path / "active_set.json"
    effect = apply_promotion_effect(
        _make_decision("promote"),
        artifact_path=artifact,
        active_set_path=asp,
    )

    assert effect["action"] == "active_set_updated"
    assert asp.is_file()
    data = json.loads(asp.read_text())
    assert data["forecaster_quantile_path"] == str(artifact)
    assert "promoted_at" in data


def test_promote_updates_existing_manifest(tmp_path) -> None:
    artifact = tmp_path / "q.joblib"
    artifact.write_bytes(b"fake")

    asp = tmp_path / "active_set.json"
    asp.write_text(json.dumps({"other_key": "existing_value"}), encoding="utf-8")

    apply_promotion_effect(_make_decision("promote"), artifact_path=artifact, active_set_path=asp)

    data = json.loads(asp.read_text())
    assert data["forecaster_quantile_path"] == str(artifact)
    # Existing keys must be preserved
    assert data["other_key"] == "existing_value"


def test_no_promote_is_noop(tmp_path) -> None:
    artifact = tmp_path / "q.joblib"
    artifact.write_bytes(b"fake")
    asp = tmp_path / "active_set.json"

    effect = apply_promotion_effect(
        _make_decision("keep_champion"),
        artifact_path=artifact,
        active_set_path=asp,
    )

    assert effect["action"] == "no_effect"
    assert not asp.exists()


def test_promote_fallback_copy_when_no_asp(tmp_path) -> None:
    artifact = tmp_path / "q.joblib"
    artifact.write_bytes(b"fake")

    effect = apply_promotion_effect(
        _make_decision("promote"),
        artifact_path=artifact,
        active_set_path=None,
    )

    # Fallback: copies to promoted_forecaster_quantile.joblib
    assert effect["action"] in ("promoted_copy", "active_set_updated")


def test_import_boundary_serving_does_not_import_training() -> None:
    """pipeline.py must not import from training_pipeline.forecaster_training at module level."""
    import importlib
    import sys

    # Remove cached modules to get a fresh import check
    for key in list(sys.modules.keys()):
        if "training_pipeline.forecaster_training" in key or "decision_engine.pipeline" in key:
            del sys.modules[key]

    # Import pipeline — should not cause training_pipeline.forecaster_training.real_data_fit to import sklearn
    import decision_engine.pipeline  # noqa: F401

    # forecaster_model.inference.quantile_infer should be importable without sklearn at module level
    import forecaster_model.inference.quantile_infer  # noqa: F401

    # The training module should NOT have been imported as a side effect of the pipeline import
    assert "training_pipeline.forecaster_training.real_data_fit" not in sys.modules

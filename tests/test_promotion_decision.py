"""FB-AUDIT-03: forecaster promotion compare."""

from __future__ import annotations

import json
from pathlib import Path

from orchestration.promotion import decide_forecaster_promotion_stub


def _report(score: float, artifact: str) -> dict:
    return {
        "forecaster_artifact": artifact,
        "best_forecaster_aggregate_score": score,
    }


def test_promote_when_no_prior() -> None:
    d = decide_forecaster_promotion_stub(report=_report(1.0, "/x.joblib"), previous_champion_path=None)
    assert d.decision == "promote"


def test_keep_champion_when_score_missing(tmp_path: Path) -> None:
    champ_dir = tmp_path / "champ"
    champ_dir.mkdir()
    (champ_dir / "forecaster_quantile_real.joblib").write_bytes(b"x")
    d = decide_forecaster_promotion_stub(
        report=_report(2.0, str(tmp_path / "new.joblib")),
        previous_champion_path=str(champ_dir / "forecaster_quantile_real.joblib"),
    )
    assert d.decision == "keep_champion"


def test_promote_when_candidate_beats_champion(tmp_path: Path) -> None:
    champ = tmp_path / "prev"
    champ.mkdir()
    (champ / "training_report.json").write_text(
        json.dumps({"best_forecaster_aggregate_score": 1.0}),
        encoding="utf-8",
    )
    d = decide_forecaster_promotion_stub(
        report=_report(1.5, str(tmp_path / "c.joblib")),
        previous_champion_path=str(champ),
    )
    assert d.decision == "promote"


def test_keep_champion_when_candidate_not_better(tmp_path: Path) -> None:
    champ = tmp_path / "prev"
    champ.mkdir()
    (champ / "training_report.json").write_text(
        json.dumps({"best_forecaster_aggregate_score": 5.0}),
        encoding="utf-8",
    )
    d = decide_forecaster_promotion_stub(
        report=_report(3.0, str(tmp_path / "c.joblib")),
        previous_champion_path=str(champ),
    )
    assert d.decision == "keep_champion"

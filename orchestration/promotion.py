"""Champion vs candidate bookkeeping (FB-AUDIT-03 / FB-SPEC-06).

Writes `promotion_decision.json` next to `training_report.json` after a campaign.
When `NM_PREVIOUS_FORECASTER_CHAMPION_PATH` points at a prior run (dir, `training_report.json`,
or `.joblib`), compares `best_forecaster_aggregate_score` and promotes only if candidate is strictly better.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from app.config.model_artifacts import load_training_report_score, resolve_champion_training_report_path

Component = Literal["forecaster", "policy"]


@dataclass
class PromotionDecision:
    """Aligned with master spec object 23 (minimal fields)."""

    component: Component
    current_champion_id: str | None
    candidate_id: str | None
    decision: Literal["promote", "keep_champion", "no_decision"]
    reasons: list[str]
    comparison_metrics: dict[str, Any]
    timestamp: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "component": self.component,
            "current_champion_id": self.current_champion_id,
            "candidate_id": self.candidate_id,
            "decision": self.decision,
            "reasons": self.reasons,
            "comparison_metrics": self.comparison_metrics,
            "timestamp": self.timestamp,
        }


def decide_forecaster_promotion_stub(
    *,
    report: dict[str, Any],
    previous_champion_path: str | None = None,
) -> PromotionDecision:
    """
    Compare candidate vs prior champion using `best_forecaster_aggregate_score` when available.

    - No prior path → **promote** (first artifact).
    - Prior path but no parsable champion score → **keep_champion** (safe default per spec §15).
    - Candidate score strictly greater than champion → **promote**; else **keep_champion**.
    """
    best = float(report.get("best_forecaster_aggregate_score", float("nan")))
    candidate_id = str(report.get("forecaster_artifact", "unknown"))
    reasons: list[str] = []
    metrics: dict[str, Any] = {
        "best_forecaster_aggregate_score": best,
        "candidate_score": best,
    }

    if not previous_champion_path or not str(previous_champion_path).strip():
        reasons.append("no prior champion path — candidate is first serving artifact")
        return PromotionDecision(
            component="forecaster",
            current_champion_id=None,
            candidate_id=candidate_id,
            decision="promote",
            reasons=reasons,
            comparison_metrics=metrics,
            timestamp=datetime.now(UTC).isoformat(),
        )

    champ_report_path = resolve_champion_training_report_path(previous_champion_path.strip())
    champion_score: float | None = None
    if champ_report_path is not None:
        champion_score = load_training_report_score(champ_report_path)
        metrics["champion_training_report"] = str(champ_report_path)
    metrics["champion_score"] = champion_score

    if champion_score is None or champion_score != champion_score:  # NaN
        reasons.append(
            "keep champion: could not read champion `best_forecaster_aggregate_score` "
            f"from {previous_champion_path!r} (point to dir with training_report.json, the report file, or joblib)"
        )
        return PromotionDecision(
            component="forecaster",
            current_champion_id=previous_champion_path,
            candidate_id=candidate_id,
            decision="keep_champion",
            reasons=reasons,
            comparison_metrics=metrics,
            timestamp=datetime.now(UTC).isoformat(),
        )

    if best > champion_score:
        reasons.append(
            f"candidate score {best} > champion score {champion_score} — promote per nightly compare"
        )
        decision: Literal["promote", "keep_champion", "no_decision"] = "promote"
    else:
        reasons.append(
            f"keep champion: candidate score {best} <= champion score {champion_score}"
        )
        decision = "keep_champion"

    return PromotionDecision(
        component="forecaster",
        current_champion_id=previous_champion_path,
        candidate_id=candidate_id,
        decision=decision,
        reasons=reasons,
        comparison_metrics=metrics,
        timestamp=datetime.now(UTC).isoformat(),
    )


def write_promotion_sidecar(artifact_dir: Path, decision: PromotionDecision) -> Path:
    path = artifact_dir / "promotion_decision.json"
    path.write_text(json.dumps(decision.to_dict(), indent=2), encoding="utf-8")
    return path

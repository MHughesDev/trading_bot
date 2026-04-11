"""Champion vs candidate bookkeeping (FB-AUDIT-03 / FB-SPEC-06 stub).

Writes `promotion_decision.json` next to `training_report.json` after a campaign.
Full gates from master spec §14–15 remain future work; this records a structured decision.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

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
    Stub: keep champion if a prior artifact path exists and new score is not strictly better.

    Operators can replace this with real gates; output is still useful for audit trails.
    """
    best = float(report.get("best_forecaster_aggregate_score", float("nan")))
    candidate_id = str(report.get("forecaster_artifact", "unknown"))
    reasons: list[str] = []
    metrics = {"best_forecaster_aggregate_score": best, "stub": True}

    if previous_champion_path and Path(previous_champion_path).is_file():
        decision: Literal["promote", "keep_champion", "no_decision"] = "keep_champion"
        reasons.append("stub: prior champion artifact exists — manual review recommended before promote")
    else:
        decision = "promote"
        reasons.append("stub: no prior champion path — candidate is first serving artifact")

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

"""APEX research experiment registry (FB-CAN-011).

Persists experiment records per
``docs/Human Provided Specs/new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md``.
Links to release candidates via ``linked_release_candidate_id`` (see ``orchestration/release_gating``).
"""

from __future__ import annotations

import hashlib
import json
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

ExperimentStatus = Literal[
    "draft",
    "running",
    "completed",
    "rejected",
    "candidate_for_shadow",
    "candidate_for_release",
    "archived",
]

ExperimentDomain = Literal[
    "signal_research",
    "trigger_research",
    "auction_research",
    "risk_sizing_research",
    "execution_research",
    "state_regime_research",
    "degradation_safety_research",
    "monitoring_alerting_research",
    "replay_simulation_methodology",
    "other",
]

ChangeType = Literal[
    "new_feature_family",
    "new_threshold_set",
    "new_weighting_scheme",
    "new_trigger_rule",
    "new_penalty_constraint",
    "new_execution_heuristic",
    "new_degradation_rule",
    "new_monitoring_rule",
    "other",
]


class ExperimentRecord(BaseModel):
    """Single experiment (spec §3)."""

    model_config = ConfigDict(extra="ignore")

    experiment_id: str
    title: str
    owner: str = ""
    created_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat()
    )
    status: ExperimentStatus = "draft"
    domain: ExperimentDomain = "other"
    hypothesis: str = ""
    motivation: str = ""
    change_type: ChangeType = "other"
    affected_components: list[str] = Field(default_factory=list)
    datasets_used: list[str] = Field(default_factory=list)
    config_versions_used: list[str] = Field(default_factory=list)
    logic_versions_used: list[str] = Field(default_factory=list)
    metrics_defined_before_run: list[str] = Field(default_factory=list)
    replay_runs: list[str] = Field(default_factory=list)
    scenario_tests: list[str] = Field(default_factory=list)
    shadow_results: str = ""
    summary_result: str = ""
    success_decision: str = ""
    failure_modes_observed: str = ""
    notes: str = ""
    linked_release_candidate: str | None = None
    tags: list[str] = Field(default_factory=list)

    def to_serializable_dict(self) -> dict[str, Any]:
        return self.model_dump(mode="json")


class ExperimentRegistry(BaseModel):
    """On-disk registry (JSON)."""

    model_config = ConfigDict(extra="ignore")

    schema_version: int = 1
    updated_at: str = Field(
        default_factory=lambda: datetime.now(UTC).replace(microsecond=0).isoformat()
    )
    experiments: list[ExperimentRecord] = Field(default_factory=list)


def default_experiment_registry_path() -> Path:
    return Path("models") / "registry" / "experiment_registry.json"


def read_experiment_registry(path: str | Path | None = None) -> ExperimentRegistry | None:
    p = Path(path) if path is not None else default_experiment_registry_path()
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(raw, dict):
        return None
    return ExperimentRegistry.model_validate(raw)


def write_experiment_registry(reg: ExperimentRegistry, path: str | Path | None = None) -> Path:
    p = Path(path) if path is not None else default_experiment_registry_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    body = reg.model_dump(mode="json")
    body["updated_at"] = datetime.now(UTC).replace(microsecond=0).isoformat()
    p.write_text(json.dumps(body, indent=2), encoding="utf-8")
    return p


def upsert_experiment(reg: ExperimentRegistry, record: ExperimentRecord) -> ExperimentRegistry:
    """Replace by ``experiment_id`` or append."""
    out = [e for e in reg.experiments if e.experiment_id != record.experiment_id]
    out.append(record)
    out.sort(key=lambda x: x.experiment_id)
    return ExperimentRegistry(schema_version=reg.schema_version, experiments=out)


def link_experiment_to_release(
    reg: ExperimentRegistry,
    *,
    experiment_id: str,
    release_candidate_id: str,
) -> ExperimentRegistry:
    """Set ``linked_release_candidate`` on a record."""
    found = False
    new_list: list[ExperimentRecord] = []
    for e in reg.experiments:
        if e.experiment_id == experiment_id:
            found = True
            new_list.append(
                e.model_copy(update={"linked_release_candidate": release_candidate_id})
            )
        else:
            new_list.append(e)
    if not found:
        raise KeyError(f"experiment_id not found: {experiment_id!r}")
    return ExperimentRegistry(schema_version=reg.schema_version, experiments=new_list)


def query_experiments(
    reg: ExperimentRegistry,
    *,
    domain: ExperimentDomain | None = None,
    status: ExperimentStatus | None = None,
    change_type: ChangeType | None = None,
    tag: str | None = None,
    component_substring: str | None = None,
    notes_substring: str | None = None,
    linked_release: str | None = None,
) -> list[ExperimentRecord]:
    """Spec §15-style filters (substring / equality)."""
    out: list[ExperimentRecord] = []
    for e in reg.experiments:
        if domain is not None and e.domain != domain:
            continue
        if status is not None and e.status != status:
            continue
        if change_type is not None and e.change_type != change_type:
            continue
        if tag is not None and tag not in (e.tags or []):
            continue
        if component_substring is not None:
            cs = component_substring.lower()
            if not any(cs in (c or "").lower() for c in e.affected_components):
                continue
        if notes_substring is not None:
            ns = notes_substring.lower()
            blob = " ".join(
                [e.notes, e.failure_modes_observed, e.summary_result, e.hypothesis]
            ).lower()
            if ns not in blob:
                continue
        if linked_release is not None and e.linked_release_candidate != linked_release:
            continue
        out.append(e)
    return out


def suggest_experiment_id(title: str, owner: str = "") -> str:
    """Stable, filesystem-friendly id: ``exp-YYYYMM-<slug>-<sha8>`` (content-based)."""
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")[:48] or "untitled"
    digest = hashlib.sha256(f"{title}\n{owner}".encode()).hexdigest()[:8]
    ym = datetime.now(UTC).strftime("%Y%m")
    return f"exp-{ym}-{slug}-{digest}"

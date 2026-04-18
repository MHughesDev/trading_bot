"""APEX research experiment registry (FB-CAN-011, FB-CAN-027).

Persists experiment records per
``docs/Human Provided Specs/new_specs/canonical/APEX_Research_Experiment_Registry_Spec_v1_0.md``.
Links to release candidates via ``linked_release_candidate`` (see ``orchestration/release_gating``).
"""

from __future__ import annotations

import hashlib
import json
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

from orchestration.release_gating import (
    ReleaseCandidate,
    ReleaseLedger,
    default_release_ledger_path,
    read_release_ledger,
    write_release_ledger,
)

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

# Spec §4 — allowed transitions (FB-CAN-027)
_EXPERIMENT_STATUS_TRANSITIONS: dict[str, frozenset[str]] = {
    "draft": frozenset({"running", "rejected", "archived"}),
    "running": frozenset({"completed", "rejected", "candidate_for_shadow", "archived"}),
    "completed": frozenset({"candidate_for_shadow", "candidate_for_release", "rejected", "archived"}),
    "candidate_for_shadow": frozenset({"candidate_for_release", "rejected", "archived"}),
    "candidate_for_release": frozenset({"archived"}),
    "rejected": frozenset({"archived"}),
    "archived": frozenset(),
}


def validate_experiment_transition(old_status: ExperimentStatus, new_status: ExperimentStatus) -> None:
    """Raise ValueError if transition is not allowed."""
    if old_status == new_status:
        return
    allowed = _EXPERIMENT_STATUS_TRANSITIONS.get(old_status)
    if allowed is None or new_status not in allowed:
        raise ValueError(
            f"invalid status transition {old_status!r} -> {new_status!r}; "
            f"allowed from {old_status!r}: {sorted(allowed or ())}"
        )


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


def validate_experiment_record_fields(record: ExperimentRecord, *, require_non_draft_fields: bool) -> list[str]:
    """Return human-readable validation errors (empty if ok)."""
    errs: list[str] = []
    if not (record.experiment_id or "").strip():
        errs.append("experiment_id is required")
    if not (record.title or "").strip():
        errs.append("title is required")
    st = record.status
    if require_non_draft_fields or st != "draft":
        if not (record.owner or "").strip():
            errs.append("owner is required when status is not draft")
        if not (record.hypothesis or "").strip():
            errs.append("hypothesis is required when status is not draft")
        if not record.metrics_defined_before_run:
            errs.append("metrics_defined_before_run must be non-empty when status is not draft")
    return errs


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


def load_or_create_experiment_registry(path: str | Path | None = None) -> ExperimentRegistry:
    """Return on-disk registry or an empty one."""
    r = read_experiment_registry(path)
    return r if r is not None else ExperimentRegistry()


def sync_experiment_release_link_in_ledger(
    *,
    experiment_id: str,
    release_candidate_id: str | None,
    ledger_path: str | Path | None = None,
) -> bool:
    """
    Keep ``ReleaseCandidate.linked_experiment_ids`` aligned with
    ``ExperimentRecord.linked_release_candidate`` (FB-CAN-027).

    Returns True if the ledger file was written. If ``release_candidate_id`` is set, that
    release must already exist in the ledger (append candidates via tooling first).
    """
    lp = Path(ledger_path) if ledger_path is not None else default_release_ledger_path()
    led = read_release_ledger(lp) or ReleaseLedger()
    if release_candidate_id is not None:
        if not any(c.release_id == release_candidate_id for c in led.candidates):
            raise ValueError(
                f"release candidate {release_candidate_id!r} not found in ledger {lp}; "
                "add the ReleaseCandidate to release_ledger.json first"
            )
    changed = False
    new_cands: list[ReleaseCandidate] = []
    for c in led.candidates:
        ids = [x for x in c.linked_experiment_ids if x != experiment_id]
        if release_candidate_id is not None and c.release_id == release_candidate_id:
            if experiment_id not in ids:
                ids.append(experiment_id)
        if ids != c.linked_experiment_ids:
            changed = True
        new_cands.append(c.model_copy(update={"linked_experiment_ids": ids}))
    if changed:
        write_release_ledger(ReleaseLedger(schema_version=led.schema_version, candidates=new_cands), lp)
    return changed


def upsert_experiment(
    reg: ExperimentRegistry,
    record: ExperimentRecord,
    *,
    ledger_path: str | Path | None = None,
) -> ExperimentRegistry:
    """Replace by ``experiment_id`` or append; validate lifecycle + required fields; sync release ledger."""
    prev: ExperimentRecord | None = None
    for e in reg.experiments:
        if e.experiment_id == record.experiment_id:
            prev = e
            break
    if prev is not None:
        validate_experiment_transition(prev.status, record.status)
    errs = validate_experiment_record_fields(record, require_non_draft_fields=False)
    if errs:
        raise ValueError("; ".join(errs))

    out = [e for e in reg.experiments if e.experiment_id != record.experiment_id]
    out.append(record)
    out.sort(key=lambda x: x.experiment_id)
    new_reg = ExperimentRegistry(schema_version=reg.schema_version, experiments=out)
    sync_experiment_release_link_in_ledger(
        experiment_id=record.experiment_id,
        release_candidate_id=record.linked_release_candidate,
        ledger_path=ledger_path,
    )
    return new_reg


def delete_experiment(
    reg: ExperimentRegistry,
    experiment_id: str,
    *,
    ledger_path: str | Path | None = None,
) -> ExperimentRegistry:
    """Remove an experiment and drop it from any release candidate links."""
    new_list = [e for e in reg.experiments if e.experiment_id != experiment_id]
    if len(new_list) == len(reg.experiments):
        raise KeyError(f"experiment_id not found: {experiment_id!r}")
    new_reg = ExperimentRegistry(schema_version=reg.schema_version, experiments=new_list)
    sync_experiment_release_link_in_ledger(
        experiment_id=experiment_id,
        release_candidate_id=None,
        ledger_path=ledger_path,
    )
    return new_reg


def link_experiment_to_release(
    reg: ExperimentRegistry,
    *,
    experiment_id: str,
    release_candidate_id: str,
    ledger_path: str | Path | None = None,
) -> ExperimentRegistry:
    """Set ``linked_release_candidate`` on a record and mirror on the release ledger."""
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
    new_reg = ExperimentRegistry(schema_version=reg.schema_version, experiments=new_list)
    sync_experiment_release_link_in_ledger(
        experiment_id=experiment_id,
        release_candidate_id=release_candidate_id,
        ledger_path=ledger_path,
    )
    return new_reg


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

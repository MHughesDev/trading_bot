"""Immutable canonical config diff audit trail and semantic guardrails (FB-CAN-057).

Appends JSON lines to ``models/registry/config_diff_audit.jsonl`` (gitignored).
See APEX Canonical Configuration spec §4 (changes must be diffable) and
Config Management spec (promotion evidence).
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from app.config.canonical_config import CanonicalRuntimeConfig
from orchestration.release_evidence import diff_canonical_runtime

DEFAULT_AUDIT_PATH = Path("models") / "registry" / "config_diff_audit.jsonl"


def _now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat()


def analyze_semantic_changes(changes: list[dict[str, Any]]) -> dict[str, Any]:
    """
    Classify diff rows into semantic categories so operators see *meaning*, not only values.

    ``changes`` items match :func:`orchestration.release_evidence.diff_canonical_runtime` output.
    """
    cats: dict[str, list[str]] = {
        "risk_sizing": [],
        "execution": [],
        "trigger": [],
        "auction": [],
        "replay_simulation": [],
        "signal_confidence": [],
        "state_safety": [],
        "shadow_comparison": [],
        "other": [],
    }
    breaking = False
    for ch in changes:
        path = str(ch.get("path") or "")
        kind = str(ch.get("kind") or "")
        if kind == "type_mismatch":
            breaking = True
        pl = path.lower()
        if pl.startswith("metadata") or pl.startswith(".metadata"):
            cats["other"].append(path)
        elif pl.startswith("domains.risk_sizing"):
            cats["risk_sizing"].append(path)
        elif pl.startswith("domains.execution"):
            cats["execution"].append(path)
        elif pl.startswith("domains.trigger"):
            cats["trigger"].append(path)
        elif pl.startswith("domains.auction"):
            cats["auction"].append(path)
        elif pl.startswith("domains.replay"):
            cats["replay_simulation"].append(path)
        elif pl.startswith("domains.signal_confidence"):
            cats["signal_confidence"].append(path)
        elif pl.startswith("domains.state_safety"):
            cats["state_safety"].append(path)
        elif pl.startswith("domains.shadow_comparison"):
            cats["shadow_comparison"].append(path)
        else:
            if path and path != "<root>":
                cats["other"].append(path)

    touched = {k: v for k, v in cats.items() if v}
    return {
        "semantic_categories": touched,
        "category_keys": sorted(touched.keys()),
        "breaking_change": breaking,
        "requires_operator_review": bool(touched.get("risk_sizing") or touched.get("execution")),
        "replay_config_touched": bool(touched.get("replay_simulation")),
    }


def replay_compatibility_hints(
    changes: list[dict[str, Any]],
    baseline: CanonicalRuntimeConfig,
    current: CanonicalRuntimeConfig,
) -> dict[str, Any]:
    """Hints for whether replay / promotion workflows need re-validation."""
    sem = analyze_semantic_changes(changes)
    bv = baseline.metadata.config_version
    cv = current.metadata.config_version
    bl = baseline.metadata.logic_version
    cl = current.metadata.logic_version
    return {
        "config_version_changed": bv != cv,
        "logic_version_changed": bl != cl,
        "replay_domain_changed": sem["replay_config_touched"],
        "recommend_full_replay_smoke": sem["replay_config_touched"] or sem["breaking_change"],
        "baseline_config_version": bv,
        "current_config_version": cv,
        "baseline_logic_version": bl,
        "current_logic_version": cl,
    }


def render_config_diff_markdown(
    diff_payload: dict[str, Any],
    semantic: dict[str, Any],
    replay: dict[str, Any],
) -> str:
    """Human-readable diff for operators (paste into PR / runbook)."""
    lines: list[str] = []
    lines.append("# Canonical config diff")
    lines.append("")
    lines.append(
        f"- Baseline **config_version**: `{diff_payload.get('baseline_config_version')}`  "
        f"→ Current: `{diff_payload.get('current_config_version')}`"
    )
    lines.append(
        f"- Baseline **logic_version**: `{diff_payload.get('baseline_logic_version')}`  "
        f"→ Current: `{diff_payload.get('current_logic_version')}`"
    )
    lines.append(f"- **Change count**: {diff_payload.get('change_count', 0)}")
    lines.append("")
    lines.append("## Semantic guardrails")
    lines.append("")
    if semantic.get("breaking_change"):
        lines.append("- **Type mismatch** detected in at least one path — treat as **breaking**.")
    else:
        lines.append("- No structural type mismatches in this diff.")
    if semantic.get("requires_operator_review"):
        lines.append("- **Risk and/or execution** domains changed — operator review recommended.")
    if semantic.get("replay_config_touched"):
        lines.append("- **Replay / simulation** domain changed — re-run replay determinism / equivalence checks.")
    lines.append("")
    lines.append("## Replay compatibility")
    lines.append("")
    lines.append(f"- `config_version_changed`: **{replay.get('config_version_changed')}**")
    lines.append(f"- `logic_version_changed`: **{replay.get('logic_version_changed')}**")
    lines.append(f"- `replay_domain_changed`: **{replay.get('replay_domain_changed')}**")
    lines.append(f"- `recommend_full_replay_smoke`: **{replay.get('recommend_full_replay_smoke')}**")
    lines.append("")
    sc = semantic.get("semantic_categories") or {}
    if sc:
        lines.append("## Changed paths by category")
        lines.append("")
        for cat in sorted(sc.keys()):
            lines.append(f"### {cat}")
            for p in sc[cat][:40]:
                lines.append(f"- `{p}`")
            if len(sc[cat]) > 40:
                lines.append(f"- … ({len(sc[cat]) - 40} more)")
            lines.append("")
    n = int(diff_payload.get("change_count") or 0)
    if n > 0:
        lines.append("## Sample changes (first 24)")
        lines.append("")
        chg = diff_payload.get("changes") or []
        for row in chg[:24]:
            p = row.get("path")
            k = row.get("kind")
            lines.append(f"- `{p}` — **{k}**")
        if n > 24:
            lines.append(f"- … ({n - 24} more rows in `changes`)")
    return "\n".join(lines).rstrip() + "\n"


def build_canonical_config_diff_report(
    baseline: CanonicalRuntimeConfig,
    current: CanonicalRuntimeConfig,
) -> dict[str, Any]:
    """Full diff + semantics + replay hints + markdown (no I/O)."""
    diff_payload = diff_canonical_runtime(baseline, current)
    changes = diff_payload.get("changes") or []
    if not isinstance(changes, list):
        changes = []
    semantic = analyze_semantic_changes(changes)
    replay = replay_compatibility_hints(changes, baseline, current)
    md = render_config_diff_markdown(diff_payload, semantic, replay)
    return {
        "schema_version": 1,
        "generated_at": _now_iso(),
        "canonical_diff": diff_payload,
        "semantic_analysis": semantic,
        "replay_compatibility": replay,
        "markdown_render": md,
    }


def append_config_diff_audit_entry(
    report: dict[str, Any],
    *,
    path: Path | str | None = None,
) -> Path:
    """Append one immutable JSON line to the audit file (creates parent dirs)."""
    p = Path(path) if path is not None else DEFAULT_AUDIT_PATH
    p.parent.mkdir(parents=True, exist_ok=True)
    line = json.dumps(report, sort_keys=True, default=str) + "\n"
    with p.open("a", encoding="utf-8") as f:
        f.write(line)
    return p


def read_config_diff_audit_tail(*, path: Path | str | None = None, limit: int = 50) -> list[dict[str, Any]]:
    """Last ``limit`` JSON objects from the audit file (oldest-first in returned list)."""
    p = Path(path) if path is not None else DEFAULT_AUDIT_PATH
    if not p.is_file():
        return []
    lines = [ln for ln in p.read_text(encoding="utf-8").splitlines() if ln.strip()]
    chunk = lines[-limit:] if limit > 0 else lines
    out: list[dict[str, Any]] = []
    for ln in chunk:
        try:
            out.append(json.loads(ln))
        except json.JSONDecodeError:
            continue
    return out


__all__ = [
    "DEFAULT_AUDIT_PATH",
    "analyze_semantic_changes",
    "append_config_diff_audit_entry",
    "build_canonical_config_diff_report",
    "read_config_diff_audit_tail",
    "render_config_diff_markdown",
    "replay_compatibility_hints",
]

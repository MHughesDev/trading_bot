"""Operator intent for ``NM_EXECUTION_MODE`` (paper vs live): persisted for next process start.

The control plane and live runtime load settings from **environment** at startup. Changing mode
therefore requires updating **``.env``** (or YAML) **and** restarting API / ``live_service`` /
``power_supervisor``. This module stores a **pending** intent in ``data/execution_profile.json``
so the UI can show *restart required* until processes pick up the new env.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

import yaml

ExecutionMode = Literal["paper", "live"]

_REPO_ROOT = Path(__file__).resolve().parents[1]
_STATE_PATH = _REPO_ROOT / "data" / "execution_profile.json"
_DEFAULT_YAML = _REPO_ROOT / "app" / "config" / "default.yaml"


def _ensure_parent() -> None:
    _STATE_PATH.parent.mkdir(parents=True, exist_ok=True)


def read_pending_intent() -> ExecutionMode | None:
    """Return pending mode from disk, or ``None`` if missing/invalid."""
    try:
        raw = _STATE_PATH.read_text(encoding="utf-8")
        data = json.loads(raw)
        m = str(data.get("execution_mode", "")).strip().lower()
        if m in ("paper", "live"):
            return m  # type: ignore[return-value]
    except (OSError, json.JSONDecodeError, TypeError):
        pass
    return None


def write_pending_intent(intent: ExecutionMode, active: ExecutionMode) -> None:
    """Persist operator **intent** vs **active** process mode. Clears file when intent matches active."""
    if intent == active:
        clear_pending_intent()
        return
    _ensure_parent()
    payload = {
        "execution_mode": intent,
        "updated_at_utc": datetime.now(UTC).isoformat(),
    }
    _STATE_PATH.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def clear_pending_intent() -> None:
    try:
        _STATE_PATH.unlink()
    except OSError:
        pass


def profile_payload(active: ExecutionMode) -> dict[str, Any]:
    """``active`` = ``settings.execution_mode`` (current process)."""
    pending = read_pending_intent()
    restart_required = pending is not None and pending != active
    return {
        "active_execution_mode": active,
        "pending_execution_mode": pending,
        "restart_required": restart_required,
        "state_path": str(_STATE_PATH),
    }


def _patch_default_yaml(mode: ExecutionMode) -> bool:
    if not _DEFAULT_YAML.is_file():
        return False
    try:
        with open(_DEFAULT_YAML, encoding="utf-8") as f:
            cfg = yaml.safe_load(f) or {}
        if "execution" not in cfg or not isinstance(cfg["execution"], dict):
            cfg["execution"] = {}
        cfg["execution"]["mode"] = mode
        with open(_DEFAULT_YAML, "w", encoding="utf-8") as f:
            yaml.safe_dump(
                cfg,
                f,
                default_flow_style=False,
                sort_keys=False,
                allow_unicode=True,
            )
        return True
    except OSError:
        return False


def _patch_dot_env(mode: ExecutionMode) -> bool:
    env_path = _REPO_ROOT / ".env"
    if not env_path.is_file():
        return False
    key = "NM_EXECUTION_MODE"
    try:
        raw = env_path.read_text(encoding="utf-8")
    except OSError:
        return False
    lines = raw.splitlines()
    out: list[str] = []
    found = False
    for line in lines:
        s = line.strip()
        if s.startswith(f"{key}=") or s.startswith(f"{key} "):
            out.append(f"{key}={mode}")
            found = True
        else:
            out.append(line)
    if not found:
        if out and out[-1].strip():
            out.append("")
        out.append(f"{key}={mode}")
    try:
        env_path.write_text("\n".join(out) + "\n", encoding="utf-8")
        return True
    except OSError:
        return False


def apply_intent_to_config_files(intent: ExecutionMode) -> dict[str, bool]:
    """Update ``default.yaml`` and ``.env`` when present. Returns which paths were written."""
    return {
        "default_yaml": _patch_default_yaml(intent),
        "dot_env": _patch_dot_env(intent),
    }

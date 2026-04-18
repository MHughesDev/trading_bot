"""Persist last shadow comparison report for governance APIs (FB-CAN-038)."""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

_DEFAULT = Path("models") / "registry" / "shadow_comparison_store.json"


def default_shadow_comparison_store_path() -> Path:
    return _DEFAULT


def load_shadow_comparison_store(path: str | Path | None = None) -> dict[str, Any] | None:
    p = Path(path) if path is not None else _DEFAULT
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return raw if isinstance(raw, dict) else None


def save_shadow_comparison_report(
    report: dict[str, Any],
    *,
    path: str | Path | None = None,
) -> Path:
    """Write report; merges into a small wrapper with updated_at."""
    p = Path(path) if path is not None else _DEFAULT
    p.parent.mkdir(parents=True, exist_ok=True)
    body = {
        "schema_version": 1,
        "updated_at": datetime.now(UTC).replace(microsecond=0).isoformat(),
        "last_report": report,
    }
    p.write_text(json.dumps(body, indent=2), encoding="utf-8")
    return p

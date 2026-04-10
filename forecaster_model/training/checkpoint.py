"""Lightweight JSON checkpointing for training metadata + paths (weights stored separately)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


def save_json_checkpoint(path: str | Path, payload: dict[str, Any]) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")


def load_json_checkpoint(path: str | Path) -> dict[str, Any]:
    p = Path(path)
    return json.loads(p.read_text(encoding="utf-8"))

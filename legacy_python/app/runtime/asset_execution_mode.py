"""Per-symbol execution mode (paper vs live) persisted on disk (FB-AP-030).

Sidecar JSON under ``data/asset_execution_mode/<symbol>.json`` — override **``NM_EXECUTION_MODE``**
for routing **that symbol's** orders only. When no file exists, :func:`effective_execution_mode`
falls back to application default from settings / env.

Does not replace global adapter env (``NM_EXECUTION_ADAPTER``): stub/mock adapters still apply
when set, via :class:`execution.service.ExecutionService` fixed-adapter injection.
"""

from __future__ import annotations

import json
import os
from datetime import UTC, datetime
from pathlib import Path
from typing import Literal

from app.config.settings import AppSettings
from app.runtime import user_data_paths as user_paths

_DEFAULT_DIR = Path(os.getenv("NM_ASSET_EXECUTION_MODE_DIR", "data/asset_execution_mode"))

ExecutionModeChoice = Literal["paper", "live"]


def mode_dir() -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        return user_paths.asset_execution_mode_dir()
    return _DEFAULT_DIR


def _path(symbol: str) -> Path:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for execution mode path")
    return mode_dir() / f"{sym}.json"


def read_mode_override(symbol: str) -> ExecutionModeChoice | None:
    """Return persisted mode, or ``None`` if unset / unreadable."""
    p = _path(symbol)
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None
    m = raw.get("execution_mode") if isinstance(raw, dict) else None
    if m == "paper" or m == "live":
        return m
    return None


def write_mode_override(symbol: str, mode: ExecutionModeChoice) -> Path:
    p = _path(symbol)
    p.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "execution_mode": mode,
        "updated_at": datetime.now(tz=UTC).isoformat().replace("+00:00", "Z"),
    }
    p.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return p


def delete_mode_override(symbol: str) -> bool:
    p = _path(symbol)
    if not p.is_file():
        return False
    p.unlink()
    return True


def effective_execution_mode(symbol: str, settings: AppSettings) -> ExecutionModeChoice:
    """Per-symbol override if set; else ``settings.execution_mode``."""
    o = read_mode_override(symbol)
    if o is not None:
        return o
    return settings.execution_mode


def to_api_dict(symbol: str, settings: AppSettings) -> dict[str, object]:
    sym = symbol.strip()
    o = read_mode_override(sym)
    return {
        "symbol": sym,
        "execution_mode": effective_execution_mode(sym, settings),
        "override": o,
        "default_execution_mode": settings.execution_mode,
    }


def list_mode_overrides() -> list[dict[str, str]]:
    """Symbols with persisted overrides (best-effort; skips invalid files)."""
    d = mode_dir()
    if not d.is_dir():
        return []
    out: list[dict[str, str]] = []
    for p in sorted(d.glob("*.json")):
        sym = p.stem
        m = read_mode_override(sym)
        if m is not None:
            out.append({"symbol": sym, "execution_mode": m})
    return out

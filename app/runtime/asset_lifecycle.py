"""Filesystem persistence for per-asset lifecycle state (FB-AP-005)."""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

from app.contracts.asset_lifecycle import AssetLifecycleRecord, AssetLifecycleState
from app.runtime.asset_model_registry import list_symbols as list_manifest_symbols, load_manifest

_DEFAULT_LIFECYCLE_DIR = Path(
    os.getenv("NM_ASSET_LIFECYCLE_DIR", "data/asset_model_registry/lifecycle")
)


def lifecycle_dir() -> Path:
    return _DEFAULT_LIFECYCLE_DIR


def _validate_symbol(symbol: str) -> str:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for lifecycle path")
    return sym


def _state_path(symbol: str) -> Path:
    return lifecycle_dir() / f"{_validate_symbol(symbol)}.json"


def load_record(symbol: str) -> AssetLifecycleRecord | None:
    """Load persisted lifecycle row; ``None`` if no file."""
    p = _state_path(symbol)
    if not p.is_file():
        return None
    raw = p.read_text(encoding="utf-8")
    return AssetLifecycleRecord.model_validate_json(raw)


def save_record(record: AssetLifecycleRecord) -> Path:
    """Atomic JSON write."""
    sym = _validate_symbol(record.symbol)
    lifecycle_dir().mkdir(parents=True, exist_ok=True)
    p = _state_path(sym)
    data = record.model_dump(mode="json")
    payload = json.dumps(data, indent=2, sort_keys=True)
    fd, tmp = tempfile.mkstemp(
        dir=p.parent, prefix=f".{p.name}.", suffix=".tmp", text=True
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(payload)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, p)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    return p


def delete_record(symbol: str) -> bool:
    p = _state_path(symbol)
    if not p.is_file():
        return False
    p.unlink()
    return True


def effective_state(symbol: str) -> AssetLifecycleState:
    """
    Effective UI/runtime state.

    If no lifecycle file exists but a model manifest exists (FB-AP-002), treat as
    ``initialized_not_active`` so existing deployments show **Start** instead of stuck **Initialize**.
    """
    rec = load_record(symbol)
    if rec is not None:
        return rec.state
    if load_manifest(symbol) is not None:
        return AssetLifecycleState.initialized_not_active
    return AssetLifecycleState.uninitialized


def list_symbols_with_records() -> list[str]:
    """Symbols that have a lifecycle JSON file."""
    d = lifecycle_dir()
    if not d.is_dir():
        return []
    out: list[str] = []
    for p in sorted(d.glob("*.json")):
        try:
            r = AssetLifecycleRecord.model_validate_json(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        out.append(r.symbol)
    return sorted(set(out))


def list_all_tracked_symbols() -> list[str]:
    """Union of lifecycle files and manifest registry symbols (for status/overview)."""
    a = set(list_symbols_with_records())
    b = set(list_manifest_symbols())
    return sorted(a | b)

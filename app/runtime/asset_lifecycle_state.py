"""Filesystem-backed per-asset lifecycle state (FB-AP-005).

Coexists with :mod:`app.runtime.asset_model_registry`: a **manifest** means artifacts exist;
**lifecycle** records whether the operator has **started** the watch loop (``active``).

* **uninitialized** — no persisted manifest for this symbol (no init completed / no manual manifest).
* **initialized_not_active** — manifest present (or explicit state file after init) but watch not started.
* **active** — operator started watch for this symbol (inference loop allowed when wired; FB-AP-038).

Default directory: ``data/asset_lifecycle_state/`` — override with **``NM_ASSET_LIFECYCLE_STATE_DIR``**.
"""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

from app.contracts.asset_lifecycle import AssetLifecycleState
from app.runtime.asset_model_registry import load_manifest

_DEFAULT_DIR = Path(os.getenv("NM_ASSET_LIFECYCLE_STATE_DIR", "data/asset_lifecycle_state"))


def state_dir() -> Path:
    return _DEFAULT_DIR


def _state_path(symbol: str) -> Path:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for lifecycle state path")
    return state_dir() / f"{sym}.json"


def _read_raw(symbol: str) -> dict | None:
    p = _state_path(symbol)
    if not p.is_file():
        return None
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None


def persisted_lifecycle_state(symbol: str) -> AssetLifecycleState | None:
    """Return state from disk only, or ``None`` if no file."""
    raw = _read_raw(symbol)
    if not raw:
        return None
    v = raw.get("lifecycle_state")
    if v is None:
        return None
    try:
        return AssetLifecycleState(str(v))
    except ValueError:
        return None


def effective_lifecycle_state(symbol: str) -> AssetLifecycleState:
    """
    Effective state for API / UI.

    Rules:
    - Without a per-asset **manifest**, the symbol is **uninitialized** (and any orphan state file
      is removed).
    - With a manifest and no state file, treat as **initialized_not_active** (manifest-only /
      post-init before Start).
    - With a manifest and a state file, use the persisted value.
    """
    sym = symbol.strip()
    has_manifest = load_manifest(sym) is not None
    persisted = persisted_lifecycle_state(sym)
    if not has_manifest:
        if persisted is not None:
            try:
                _state_path(sym).unlink()
            except OSError:
                pass
        return AssetLifecycleState.uninitialized
    if persisted is None:
        return AssetLifecycleState.initialized_not_active
    return persisted


def _atomic_write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(payload, indent=2, sort_keys=True)
    fd, tmp = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp", text=True
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def save_lifecycle_state(symbol: str, state: AssetLifecycleState) -> Path:
    """Persist lifecycle state (atomic write)."""
    from datetime import UTC, datetime

    sym = symbol.strip()
    p = _state_path(sym)
    payload = {
        "lifecycle_state": state.value,
        "updated_at": datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
    }
    _atomic_write_json(p, payload)
    return p


def delete_lifecycle_state(symbol: str) -> bool:
    p = _state_path(symbol)
    if not p.is_file():
        return False
    p.unlink()
    return True


def set_initialized_not_active(symbol: str) -> Path:
    """After successful init pipeline (FB-AP-006) or equivalent — ready to Start."""
    return save_lifecycle_state(symbol, AssetLifecycleState.initialized_not_active)


def set_active(symbol: str) -> Path:
    return save_lifecycle_state(symbol, AssetLifecycleState.active)


def transition_start(symbol: str) -> Path:
    """
    **Start** — ``initialized_not_active`` → ``active``. Requires a per-asset manifest.
    """
    sym = symbol.strip()
    if load_manifest(sym) is None:
        raise ValueError("cannot start: no model manifest for symbol (initialize first)")
    cur = effective_lifecycle_state(sym)
    if cur == AssetLifecycleState.uninitialized:
        raise ValueError("cannot start: symbol is uninitialized")
    if cur == AssetLifecycleState.active:
        raise ValueError("already active")
    return set_active(sym)


def transition_stop(symbol: str) -> Path:
    """
    **Stop** — ``active`` → ``initialized_not_active`` (caller runs flatten first — FB-AP-032).
    """
    sym = symbol.strip()
    cur = effective_lifecycle_state(sym)
    if cur != AssetLifecycleState.active:
        raise ValueError("not active")
    return set_initialized_not_active(sym)


def list_state_symbols() -> list[str]:
    """Symbols that have a lifecycle JSON file (may include orphans without manifests)."""
    d = state_dir()
    if not d.is_dir():
        return []
    out: list[str] = []
    for p in sorted(d.glob("*.json")):
        out.append(p.stem)
    return out


def lifecycle_overview() -> dict[str, str]:
    """Map symbol → effective lifecycle state string for symbols with manifest or state file."""
    from app.runtime.asset_model_registry import list_symbols as list_manifest_symbols

    syms = set(list_manifest_symbols()) | set(list_state_symbols())
    return {s: effective_lifecycle_state(s).value for s in sorted(syms)}

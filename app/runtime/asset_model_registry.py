"""Filesystem-backed per-asset model registry (FB-AP-002)."""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import user_data_paths as user_paths

_DEFAULT_DIR = Path(os.getenv("NM_ASSET_MODEL_REGISTRY_DIR", "data/asset_model_registry/manifests"))


def registry_dir() -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        return user_paths.registry_manifests_dir()
    return _DEFAULT_DIR


def _manifest_path(symbol: str) -> Path:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for registry path")
    return registry_dir() / f"{sym}.json"


def validate_manifest_symbol(request_symbol: str, manifest: AssetModelManifest) -> None:
    """Ensure manifest is not applied to the wrong symbol (FB-AP-001)."""
    a = request_symbol.strip()
    b = manifest.canonical_symbol.strip()
    if a != b:
        raise ValueError(
            f"manifest canonical_symbol {b!r} does not match request symbol {a!r}"
        )


def load_manifest(symbol: str) -> AssetModelManifest | None:
    p = _manifest_path(symbol)
    if not p.is_file():
        return None
    raw = p.read_text(encoding="utf-8")
    m = AssetModelManifest.model_validate_json(raw)
    validate_manifest_symbol(symbol, m)
    return m


def save_manifest(manifest: AssetModelManifest) -> Path:
    """Atomic write: temp + replace."""
    registry_dir().mkdir(parents=True, exist_ok=True)
    p = _manifest_path(manifest.canonical_symbol)
    data = manifest.model_dump(mode="json")
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


def list_symbols() -> list[str]:
    if not registry_dir().is_dir():
        return []
    out: list[str] = []
    for p in sorted(registry_dir().glob("*.json")):
        try:
            m = AssetModelManifest.model_validate_json(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        out.append(m.canonical_symbol)
    return sorted(set(out))


def delete_manifest(symbol: str) -> bool:
    p = _manifest_path(symbol)
    if not p.is_file():
        return False
    p.unlink()
    return True

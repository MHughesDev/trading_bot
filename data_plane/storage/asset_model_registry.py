"""Filesystem persistence for per-asset model manifests (FB-AP-002): atomic JSON writes, list/read."""

from __future__ import annotations

import json
import re
from pathlib import Path

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifestV1

_SAFE_SYMBOL_RE = re.compile(r"[^A-Za-z0-9._-]+")


def manifest_filename(symbol: str) -> str:
    """Stable filename slug for one manifest file per symbol."""
    s = symbol.strip()
    slug = _SAFE_SYMBOL_RE.sub("_", s)
    return f"{slug}.json"


def default_registry_dir() -> Path:
    return Path("data") / "asset_model_registry"


def manifest_path(registry_dir: Path, symbol: str) -> Path:
    return registry_dir / manifest_filename(symbol)


def load_manifest_file(path: Path) -> AssetModelManifestV1:
    """Load and validate a manifest JSON file."""
    raw = json.loads(path.read_text(encoding="utf-8"))
    return AssetModelManifestV1.model_validate(raw)


def read_manifest(registry_dir: Path, symbol: str) -> AssetModelManifestV1 | None:
    """Load manifest for `symbol`, or None if missing."""
    path = manifest_path(registry_dir, symbol)
    if not path.is_file():
        return None
    return load_manifest_file(path)


def resolve_manifest_for_symbol(settings: AppSettings, symbol: str) -> AssetModelManifestV1 | None:
    """
    When `NM_ASSET_MODEL_MANIFEST_PATH` or `NM_ASSET_MODEL_REGISTRY_PATH` is set, load the manifest
    for binding checks (FB-AP-003). Otherwise returns None (global artifact paths only).
    """
    single = settings.asset_model_manifest_path
    if single:
        p = Path(single)
        if not p.is_file():
            return None
        return load_manifest_file(p)
    reg = settings.asset_model_registry_path
    if reg:
        return read_manifest(Path(reg), symbol)
    return None


def list_manifests(registry_dir: Path) -> list[AssetModelManifestV1]:
    """All valid manifests in the registry directory (skips invalid JSON)."""
    if not registry_dir.is_dir():
        return []
    out: list[AssetModelManifestV1] = []
    for p in sorted(registry_dir.glob("*.json")):
        try:
            raw = json.loads(p.read_text(encoding="utf-8"))
            out.append(AssetModelManifestV1.model_validate(raw))
        except (OSError, json.JSONDecodeError, ValueError):
            continue
    return out


def write_manifest_atomic(registry_dir: Path, manifest: AssetModelManifestV1) -> Path:
    """
    Atomically write manifest JSON (write temp + replace) so readers never see partial files.
    Creates `registry_dir` if needed.
    """
    registry_dir.mkdir(parents=True, exist_ok=True)
    path = manifest_path(registry_dir, manifest.symbol)
    tmp = path.with_suffix(path.suffix + ".tmp")
    data = manifest.model_dump(mode="json")
    tmp.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp.replace(path)
    return path


def upsert_manifest(registry_dir: Path, manifest: AssetModelManifestV1) -> Path:
    """Create or replace manifest for `manifest.symbol`."""
    return write_manifest_atomic(registry_dir, manifest)

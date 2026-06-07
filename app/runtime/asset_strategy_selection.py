"""Per-symbol selected strategy, persisted on disk (FB-AP-XXX strategy-based runtime).

Sidecar JSON under ``data/asset_strategy/<symbol>.json`` — the strategy chosen on the Asset
page's backtest panel doubles as **the live decision source** for that symbol: the strategy
runs continuously (paper or live) the moment a user picks it, no separate "enable" step.

When no override is on disk, :func:`effective_strategy_for_symbol` falls back to the first
strategy registered in :mod:`strategies.registry`.
"""

from __future__ import annotations

import json
import os
from datetime import UTC, datetime
from pathlib import Path

from app.config.settings import AppSettings
from app.runtime import user_data_paths as user_paths
from strategies.registry import list_strategies

_DEFAULT_DIR = Path(os.getenv("NM_ASSET_STRATEGY_DIR", "data/asset_strategy"))


def strategy_dir() -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        return user_paths.asset_strategy_dir()
    return _DEFAULT_DIR


def _path(symbol: str) -> Path:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for strategy selection path")
    return strategy_dir() / f"{sym}.json"


def default_strategy_key() -> str | None:
    """First registered strategy key, or ``None`` if the catalogue is empty."""
    strategies = list_strategies()
    return strategies[0].key if strategies else None


def read_strategy_override(symbol: str) -> str | None:
    """Return the persisted strategy key, or ``None`` if unset / unreadable."""
    p = _path(symbol)
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None
    key = raw.get("strategy_key") if isinstance(raw, dict) else None
    return key if isinstance(key, str) and key.strip() else None


def write_strategy_override(symbol: str, strategy_key: str, *, params: dict | None = None) -> Path:
    p = _path(symbol)
    p.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "strategy_key": strategy_key,
        "strategy_params": params or {},
        "updated_at": datetime.now(tz=UTC).isoformat().replace("+00:00", "Z"),
    }
    p.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return p


def delete_strategy_override(symbol: str) -> bool:
    p = _path(symbol)
    if not p.is_file():
        return False
    p.unlink()
    return True


def read_strategy_params(symbol: str) -> dict:
    p = _path(symbol)
    if not p.is_file():
        return {}
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return {}
    params = raw.get("strategy_params") if isinstance(raw, dict) else None
    return dict(params) if isinstance(params, dict) else {}


def effective_strategy_for_symbol(symbol: str, settings: AppSettings | None = None) -> str | None:
    """Per-symbol override if set; else the catalogue's default strategy."""
    override = read_strategy_override(symbol)
    if override is not None:
        return override
    return default_strategy_key()


def to_api_dict(symbol: str) -> dict[str, object]:
    sym = symbol.strip()
    override = read_strategy_override(sym)
    return {
        "symbol": sym,
        "strategy_key": effective_strategy_for_symbol(sym),
        "override": override,
        "strategy_params": read_strategy_params(sym),
        "default_strategy_key": default_strategy_key(),
    }


def list_strategy_overrides() -> list[dict[str, str]]:
    """Symbols with persisted overrides (best-effort; skips invalid files)."""
    d = strategy_dir()
    if not d.is_dir():
        return []
    out: list[dict[str, str]] = []
    for p in sorted(d.glob("*.json")):
        sym = p.stem
        key = read_strategy_override(sym)
        if key is not None:
            out.append({"symbol": sym, "strategy_key": key})
    return out

"""Per-user ``data/users/<id>/...`` layout (FB-UX-007). Shared Kraken / QuestDB paths unchanged."""

from __future__ import annotations

import os
from pathlib import Path

from app.runtime.tenant_context import get_current_user_id

_USERS_ROOT = Path(os.getenv("NM_USER_DATA_ROOT", "data/users"))


def users_root() -> Path:
    return _USERS_ROOT


def user_data_root(user_id: int) -> Path:
    if user_id <= 0:
        raise ValueError("invalid user_id")
    return _USERS_ROOT / str(int(user_id))


def _under_data(env_key: str, default_under_data: str) -> Path:
    """
    Resolve ``NM_*`` path that lives under ``data/`` to a per-user path when
    :func:`get_current_user_id` is set.

    If ``NM_*`` is absolute, returns it unchanged (single-tenant).
    """
    raw = os.getenv(env_key, default_under_data)
    p = Path(raw)
    if p.is_absolute():
        return p
    if not str(p).startswith("data") and not str(p).startswith("data/"):
        p = Path("data") / p
    try:
        rel = p.relative_to(Path("data"))
    except ValueError:
        return p
    uid = get_current_user_id()
    if uid is None:
        return Path("data") / rel
    return user_data_root(uid) / rel


def registry_manifests_dir() -> Path:
    return _under_data("NM_ASSET_MODEL_REGISTRY_DIR", "data/asset_model_registry/manifests")


def lifecycle_state_dir() -> Path:
    return _under_data("NM_ASSET_LIFECYCLE_STATE_DIR", "data/asset_lifecycle_state")


def asset_execution_mode_dir() -> Path:
    return _under_data("NM_ASSET_EXECUTION_MODE_DIR", "data/asset_execution_mode")


def canonical_bar_watermark_dir() -> Path:
    return _under_data("NM_CANONICAL_BAR_WATERMARK_DIR", "data/canonical_bar_watermarks")


def pnl_ledger_path() -> Path:
    return _under_data("NM_PNL_LEDGER_PATH", "data/pnl_ledger.jsonl")


def trade_markers_path() -> Path:
    return _under_data("NM_TRADE_MARKERS_PATH", "data/trade_markers.jsonl")


def asset_init_artifacts_base(settings_base: Path) -> Path:
    """``settings.asset_init_artifacts_dir`` may be ``data/asset_init`` — scope when tenant active."""
    base = Path(settings_base)
    if base.is_absolute():
        return base
    if not str(base).startswith("data") and not str(base).startswith("data/"):
        base = Path("data") / base
    try:
        rel = base.relative_to(Path("data"))
    except ValueError:
        return base
    uid = get_current_user_id()
    if uid is None:
        return Path("data") / rel
    return user_data_root(uid) / rel

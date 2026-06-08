"""Sidecar JSON: last canonical bar bucket written per symbol (FB-AP-019)."""

from __future__ import annotations

import json
import os
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from app.runtime import user_data_paths as user_paths

_DEFAULT_DIR = Path(
    os.getenv("NM_CANONICAL_BAR_WATERMARK_DIR", "data/canonical_bar_watermarks")
)


def watermark_dir() -> Path:
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        return user_paths.canonical_bar_watermark_dir()
    return _DEFAULT_DIR


def _path(symbol: str) -> Path:
    sym = symbol.strip()
    if not sym or "/" in sym or "\\" in sym or sym.startswith("."):
        raise ValueError("invalid symbol for watermark path")
    return watermark_dir() / f"{sym}.json"


def read_canonical_through(symbol: str) -> datetime | None:
    """Latest stored bucket-start UTC from sidecar, or ``None``."""
    p = _path(symbol)
    if not p.is_file():
        return None
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None
    ts = raw.get("canonical_through_ts")
    if not ts:
        return None
    try:
        t = datetime.fromisoformat(str(ts).replace("Z", "+00:00"))
    except ValueError:
        return None
    return t if t.tzinfo else t.replace(tzinfo=UTC)


def write_canonical_through(
    symbol: str,
    *,
    canonical_through_ts: datetime,
    interval_seconds: int,
) -> Path:
    """Atomic write of max bucket start we have persisted for ``symbol``."""
    t = canonical_through_ts.astimezone(UTC)
    payload = {
        "canonical_symbol": symbol.strip(),
        "interval_seconds": int(interval_seconds),
        "canonical_through_ts": t.isoformat(),
        "updated_at": datetime.now(UTC).isoformat(),
    }
    p = _path(symbol)
    p.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(payload, indent=2, sort_keys=True)
    fd, tmp = tempfile.mkstemp(
        dir=p.parent, prefix=f".{p.name}.", suffix=".tmp", text=True
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
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

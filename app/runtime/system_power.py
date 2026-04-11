"""Global system power: ON = normal operation; OFF = hard stop (no inference, trading, training).

State is persisted under ``data/system_power.json`` so restarts and the control plane stay aligned.
"""

from __future__ import annotations

import json
import threading
from pathlib import Path
from typing import Literal

PowerState = Literal["on", "off"]

_LOCK = threading.RLock()
_STATE_PATH = Path(__file__).resolve().parents[2] / "data" / "system_power.json"
_power: PowerState = "on"


def _default_from_env() -> PowerState:
    import os

    v = os.getenv("NM_SYSTEM_POWER", "on").strip().lower()
    return "off" if v in ("0", "false", "off", "no") else "on"


def _load_file() -> PowerState | None:
    try:
        raw = _STATE_PATH.read_text(encoding="utf-8")
        data = json.loads(raw)
        p = str(data.get("power", "on")).strip().lower()
        return "off" if p in ("off", "false", "0") else "on"
    except (OSError, json.JSONDecodeError, TypeError):
        return None


def _save_file(power: PowerState) -> None:
    _STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    _STATE_PATH.write_text(json.dumps({"power": power}, indent=2), encoding="utf-8")


def _init() -> None:
    global _power
    with _LOCK:
        f = _load_file()
        _power = f if f is not None else _default_from_env()


_init()


def get_power() -> PowerState:
    """Return current power state (``on`` or ``off``)."""
    with _LOCK:
        return _power


def is_on() -> bool:
    return get_power() == "on"


def set_power(power: PowerState) -> PowerState:
    """Set power and persist. Returns the new state."""
    global _power
    with _LOCK:
        p: PowerState = "off" if str(power).lower() in ("off", "false", "0") else "on"
        _power = p
        _save_file(_power)
        return _power


def sync_from_disk() -> PowerState:
    """Reload from disk (e.g. after external edit)."""
    global _power
    with _LOCK:
        f = _load_file()
        if f is not None:
            _power = f
        return _power

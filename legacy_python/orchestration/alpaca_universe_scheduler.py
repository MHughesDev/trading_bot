"""Background thread: periodic Alpaca tradable-universe sync (FB-AP-020)."""

from __future__ import annotations

import logging
import threading
from datetime import UTC, datetime
from typing import Any

from app.config.settings import AppSettings, load_settings
from orchestration.alpaca_universe_sync import sync_alpaca_tradable_universe

logger = logging.getLogger(__name__)

_lock = threading.Lock()
_thread: threading.Thread | None = None
_stop = threading.Event()
_state: dict[str, Any] = {
    "running": False,
    "last_tick_utc": None,
    "last_error": None,
    "last_count": None,
}


def alpaca_universe_scheduler_status() -> dict[str, Any]:
    with _lock:
        alive = _thread is not None and _thread.is_alive()
        return {
            "enabled": bool(_state.get("config_enabled")),
            "interval_seconds": int(_state.get("interval_seconds") or 0),
            "running": alive,
            "last_tick_utc": _state.get("last_tick_utc"),
            "last_error": _state.get("last_error"),
            "last_count": _state.get("last_count"),
        }


def _iso_z(dt: datetime) -> str:
    return dt.isoformat().replace("+00:00", "Z")


def _loop(settings: AppSettings, interval_s: float) -> None:
    while not _stop.is_set():
        try:
            _stop.wait(timeout=interval_s)
            if _stop.is_set():
                break
            with _lock:
                _state["last_tick_utc"] = _iso_z(datetime.now(tz=UTC))
            rep = sync_alpaca_tradable_universe(settings)
            with _lock:
                _state["last_error"] = rep.get("error")
                _state["last_count"] = rep.get("count")
        except Exception as e:
            logger.exception("alpaca universe scheduler tick failed")
            with _lock:
                _state["last_error"] = str(e)


def start_alpaca_universe_scheduler(s: AppSettings | None = None) -> None:
    global _thread, _stop
    cfg = s or load_settings()
    if not cfg.alpaca_universe_sync_enabled:
        with _lock:
            _state["config_enabled"] = False
            _state["interval_seconds"] = cfg.alpaca_universe_sync_interval_seconds
        logger.info("alpaca universe scheduler: disabled (NM_ALPACA_UNIVERSE_SYNC_ENABLED=false)")
        return

    interval = max(60.0, float(cfg.alpaca_universe_sync_interval_seconds))
    with _lock:
        if _thread is not None and _thread.is_alive():
            logger.info("alpaca universe scheduler: already running")
            return
        _state["config_enabled"] = True
        _state["interval_seconds"] = int(interval)
        _stop = threading.Event()

    def _target() -> None:
        try:
            sync_alpaca_tradable_universe(cfg)
            _loop(cfg, interval)
        finally:
            with _lock:
                _state["running"] = False

    with _lock:
        _state["running"] = True
    _thread = threading.Thread(
        target=_target,
        name="alpaca-universe-sync",
        daemon=True,
    )
    _thread.start()
    logger.info(
        "alpaca universe scheduler: started interval_seconds=%s",
        int(interval),
    )


def stop_alpaca_universe_scheduler() -> None:
    global _thread
    _stop.set()
    t = _thread
    if t is not None and t.is_alive():
        t.join(timeout=5.0)
    _thread = None
    with _lock:
        _state["running"] = False
    logger.info("alpaca universe scheduler: stopped")


def reset_alpaca_universe_scheduler_for_tests() -> None:
    global _thread, _stop
    _stop.set()
    if _thread is not None and _thread.is_alive():
        _thread.join(timeout=2.0)
    _thread = None
    _stop = threading.Event()
    with _lock:
        _state.clear()
        _state.update(
            {
                "running": False,
                "last_tick_utc": None,
                "last_error": None,
                "last_count": None,
            }
        )

"""
In-process background jobs while the **application** is running (FB-AP-035).

Schedulers register only when the FastAPI control plane process starts (lifespan) and stop on
shutdown. **No** training runs when the API process is not running (e.g. desktop `.exe` closed).

Nightly training uses :func:`orchestration.nightly_retrain.run_nightly_training_job` (real-data
campaign) when ``NM_SCHEDULER_NIGHTLY_ENABLED`` is true.
"""

from __future__ import annotations

import logging
import threading
from datetime import UTC, datetime, timedelta
from typing import Any

from app.config.settings import AppSettings, load_settings

logger = logging.getLogger(__name__)

_lock = threading.Lock()
_thread: threading.Thread | None = None
_stop = threading.Event()
_state: dict[str, Any] = {
    "running": False,
    "last_tick_utc": None,
    "last_run_finished_utc": None,
    "next_run_after_utc": None,
    "last_report": None,
    "last_error": None,
}


def _iso_z(dt: datetime) -> str:
    return dt.isoformat().replace("+00:00", "Z")


def _mark_run_finished() -> None:
    """Record job end time and projected next wake (interval after finish). FB-UX-012."""
    now = datetime.now(tz=UTC)
    with _lock:
        _state["last_run_finished_utc"] = _iso_z(now)
        interval = max(1, int(_state.get("interval_seconds") or 60))
        _state["next_run_after_utc"] = _iso_z(now + timedelta(seconds=interval))


def scheduler_status(*, include_report: bool = False) -> dict[str, Any]:
    """Snapshot for ``GET /status`` → ``app_scheduler``; optional full ``last_report`` for detail API."""
    with _lock:
        running = _thread is not None and _thread.is_alive()
        out: dict[str, Any] = {
            "enabled": bool(_state.get("config_enabled")),
            "interval_seconds": int(_state.get("interval_seconds") or 0),
            "running": running,
            "last_tick_utc": _state.get("last_tick_utc"),
            "last_run_finished_utc": _state.get("last_run_finished_utc"),
            "next_run_after_utc": _state.get("next_run_after_utc"),
            "last_error": _state.get("last_error"),
        }
        if include_report:
            out["last_report"] = _state.get("last_report")
        return out


def nightly_scheduler_detail(settings: AppSettings) -> dict[str, Any]:
    """Payload for ``GET /scheduler/nightly`` (FB-UX-012): status + last report + RL/forecaster flags."""
    out = dict(scheduler_status(include_report=True))
    out["nightly_per_asset_forecaster"] = bool(settings.scheduler_nightly_per_asset_forecaster)
    out["nightly_rl_requires_trade"] = bool(settings.scheduler_nightly_rl_requires_trade)
    out["nightly_rl_trade_lookback_days"] = settings.scheduler_nightly_rl_trade_lookback_days
    return out


def _loop_body(settings: AppSettings) -> None:
    from orchestration.nightly_retrain import run_nightly_training_job

    try:
        report = run_nightly_training_job(settings=settings)
        with _lock:
            _state["last_report"] = report
            _state["last_error"] = None
        logger.info("scheduler nightly job finished keys=%s", list(report.keys()) if report else [])
    except Exception as e:
        logger.exception("scheduler nightly job failed")
        with _lock:
            _state["last_error"] = str(e)
    finally:
        _mark_run_finished()


def _run(settings: AppSettings, interval_s: float) -> None:
    while not _stop.is_set():
        try:
            _stop.wait(timeout=interval_s)
            if _stop.is_set():
                break
            with _lock:
                _state["last_tick_utc"] = datetime.now(tz=UTC).isoformat().replace("+00:00", "Z")
            _loop_body(settings)
        except Exception:
            logger.exception("scheduler loop iteration failed")


def start_app_background_scheduler(s: AppSettings | None = None) -> None:
    """Start background thread if configured and not already running."""
    global _thread, _stop
    cfg = s or load_settings()
    if not cfg.scheduler_nightly_enabled:
        with _lock:
            _state["config_enabled"] = False
            _state["interval_seconds"] = cfg.scheduler_nightly_interval_seconds
            _state["next_run_after_utc"] = None
        logger.info("app scheduler: disabled (NM_SCHEDULER_NIGHTLY_ENABLED=false)")
        return

    interval = max(1.0, float(cfg.scheduler_nightly_interval_seconds))
    with _lock:
        if _thread is not None and _thread.is_alive():
            logger.info("app scheduler: already running")
            return
        _state["config_enabled"] = True
        _state["interval_seconds"] = cfg.scheduler_nightly_interval_seconds
        _stop = threading.Event()
        _state["next_run_after_utc"] = _iso_z(datetime.now(tz=UTC) + timedelta(seconds=interval))

    def _target() -> None:
        try:
            _run(cfg, interval)
        finally:
            with _lock:
                _state["running"] = False

    with _lock:
        _state["running"] = True
    _thread = threading.Thread(
        target=_target,
        name="app-nightly-scheduler",
        daemon=True,
    )
    _thread.start()
    logger.info(
        "app scheduler: started interval_seconds=%s nightly_enabled=%s",
        int(interval),
        cfg.scheduler_nightly_enabled,
    )


def stop_app_background_scheduler() -> None:
    """Signal scheduler thread to exit and join (best-effort)."""
    global _thread
    _stop.set()
    t = _thread
    if t is not None and t.is_alive():
        t.join(timeout=5.0)
    _thread = None
    with _lock:
        _state["running"] = False
    logger.info("app scheduler: stopped")


def reset_app_scheduler_for_tests() -> None:
    """Clear state for unit tests."""
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
                "last_run_finished_utc": None,
                "next_run_after_utc": None,
                "last_report": None,
                "last_error": None,
            }
        )

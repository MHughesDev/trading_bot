"""
Per-asset initialization pipeline orchestrator (FB-AP-006).

Runs **one symbol at a time** (global lock) in a background thread so the control plane
does not block. Step **kraken_fetch** performs a real Kraken REST pull via
:func:`orchestration.real_data_bars.fetch_symbol_bars_sync`. Later steps (**validate** … **register**)
are stubbed until FB-AP-007–012 wire full logic; progress is still recorded for the UI/API.
"""

from __future__ import annotations

import logging
import threading
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from enum import Enum
from typing import Any, Callable

from app.config.settings import load_settings

logger = logging.getLogger(__name__)

# Bootstrap window for first init (FB-AP-007 will refine range/granularity).
INIT_LOOKBACK_DAYS = 7
INIT_GRANULARITY_SECONDS = 60

_jobs_lock = threading.Lock()
_state_lock = threading.Lock()
_pipeline_running = False
_jobs: dict[str, dict[str, Any]] = {}


class InitPipelineStep(str, Enum):
    kraken_fetch = "kraken_fetch"
    validate = "validate"
    features = "features"
    forecaster_train = "forecaster_train"
    rl_init = "rl_init"
    register = "register"


@dataclass
class StepRecord:
    step: InitPipelineStep
    status: str  # "done" | "skipped" | "failed"
    detail: str | None
    started_at: str
    finished_at: str


def _utc_now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _new_job(symbol: str) -> str:
    job_id = str(uuid.uuid4())
    sym = symbol.strip()
    with _jobs_lock:
        _jobs[job_id] = {
            "job_id": job_id,
            "symbol": sym,
            "status": "pending",
            "error": None,
            "created_at": _utc_now_iso(),
            "started_at": None,
            "finished_at": None,
            "steps": [],
        }
    return job_id


def get_job(job_id: str) -> dict[str, Any] | None:
    with _jobs_lock:
        j = _jobs.get(job_id)
        return dict(j) if j else None


def _update_job(job_id: str, **kwargs: Any) -> None:
    with _jobs_lock:
        if job_id in _jobs:
            _jobs[job_id].update(kwargs)


def _append_step(job_id: str, rec: StepRecord) -> None:
    with _jobs_lock:
        if job_id in _jobs:
            _jobs[job_id]["steps"].append(
                {
                    "step": rec.step.value,
                    "status": rec.status,
                    "detail": rec.detail,
                    "started_at": rec.started_at,
                    "finished_at": rec.finished_at,
                }
            )


def _run_step(
    job_id: str,
    step: InitPipelineStep,
    fn: Callable[[], tuple[str, str | None]],
) -> None:
    t0 = _utc_now_iso()
    try:
        status, detail = fn()
        t1 = _utc_now_iso()
        _append_step(
            job_id,
            StepRecord(step=step, status=status, detail=detail, started_at=t0, finished_at=t1),
        )
        if status == "failed":
            raise RuntimeError(detail or step.value)
    except Exception as exc:
        t1 = _utc_now_iso()
        _append_step(
            job_id,
            StepRecord(
                step=step,
                status="failed",
                detail=str(exc),
                started_at=t0,
                finished_at=t1,
            ),
        )
        raise


def _pipeline_body(job_id: str, symbol: str) -> None:
    from orchestration.real_data_bars import fetch_symbol_bars_sync

    settings = load_settings()
    g = max(1, int(settings.training_data_granularity_seconds))
    gran = INIT_GRANULARITY_SECONDS if INIT_GRANULARITY_SECONDS >= g else g
    end = datetime.now(UTC)
    start = end - timedelta(days=INIT_LOOKBACK_DAYS)

    def do_fetch() -> tuple[str, str | None]:
        df = fetch_symbol_bars_sync(symbol, start, end, granularity_seconds=gran)
        n = df.height
        return "done", f"fetched {n} rows ({INIT_LOOKBACK_DAYS}d, {gran}s bars)"

    def do_validate() -> tuple[str, str | None]:
        return "skipped", "FB-AP-008 (clean/validate) not wired yet"

    def do_features() -> tuple[str, str | None]:
        return "skipped", "FB-AP-009 (features) not wired yet"

    def do_forecaster() -> tuple[str, str | None]:
        return "skipped", "FB-AP-010 (forecaster train) not wired yet"

    def do_rl() -> tuple[str, str | None]:
        return "skipped", "FB-AP-011 (RL init) not wired yet"

    def do_register() -> tuple[str, str | None]:
        return "skipped", "FB-AP-012 (manifest register) not wired yet"

    _run_step(job_id, InitPipelineStep.kraken_fetch, do_fetch)
    _run_step(job_id, InitPipelineStep.validate, do_validate)
    _run_step(job_id, InitPipelineStep.features, do_features)
    _run_step(job_id, InitPipelineStep.forecaster_train, do_forecaster)
    _run_step(job_id, InitPipelineStep.rl_init, do_rl)
    _run_step(job_id, InitPipelineStep.register, do_register)


def _run_pipeline(job_id: str, symbol: str) -> None:
    _update_job(job_id, status="running", started_at=_utc_now_iso())
    try:
        _pipeline_body(job_id, symbol)
        _update_job(job_id, status="succeeded", finished_at=_utc_now_iso(), error=None)
    except Exception as exc:
        logger.exception("asset init pipeline failed job_id=%s symbol=%s", job_id, symbol)
        _update_job(
            job_id,
            status="failed",
            finished_at=_utc_now_iso(),
            error=str(exc),
        )


def start_asset_init_job(symbol: str) -> str:
    """
    Start a background init job. Only one pipeline run at a time globally.

    Raises:
        RuntimeError: if another init job is already running.
    """
    global _pipeline_running
    with _state_lock:
        if _pipeline_running:
            raise RuntimeError("init pipeline already running for another symbol")
        _pipeline_running = True

    job_id = _new_job(symbol)

    def _target() -> None:
        global _pipeline_running
        try:
            _run_pipeline(job_id, symbol)
        finally:
            with _state_lock:
                _pipeline_running = False

    threading.Thread(target=_target, name=f"asset-init-{symbol.strip()}", daemon=True).start()
    return job_id


def try_start_asset_init_job(symbol: str) -> str | None:
    """Start job if runner is free; return ``None`` if busy."""
    try:
        return start_asset_init_job(symbol)
    except RuntimeError:
        return None


def reset_asset_init_pipeline_for_tests() -> None:
    """Clear job table and running flag — **tests only**."""
    global _jobs, _pipeline_running
    with _jobs_lock:
        _jobs.clear()
    with _state_lock:
        _pipeline_running = False

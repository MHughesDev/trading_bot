"""
Per-asset initialization pipeline (FB-AP-006+).

Runs **one symbol at a time** in a background thread. Steps: **kraken_fetch** (FB-AP-007),
**validate** (FB-AP-008), **features** (FB-AP-009 — Parquet artifacts for training), then stubs
for forecaster / RL / register until FB-AP-010–012.
"""

from __future__ import annotations

import json
import logging
import threading
import uuid
from dataclasses import dataclass
from enum import Enum
from typing import Any, Callable

logger = logging.getLogger(__name__)

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
    status: str
    detail: str | None
    started_at: str
    finished_at: str


def _utc_now_iso() -> str:
    from datetime import UTC, datetime

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
    from app.config.settings import load_settings
    from data_plane.bootstrap_bars import (
        InitBootstrapValidationResult,
        init_bootstrap_validation_detail_payload,
        validate_and_clean_init_bootstrap_bars,
    )
    from orchestration.init_feature_artifacts import (
        init_features_detail_payload,
        write_init_feature_artifacts,
    )
    from orchestration.init_kraken_historical import (
        InitKrakenHistoricalResult,
        fetch_init_bootstrap_bars,
        init_bootstrap_detail_payload,
    )

    settings = load_settings()
    fetch_result: InitKrakenHistoricalResult | None = None
    validation_result: InitBootstrapValidationResult | None = None

    def do_fetch() -> tuple[str, str | None]:
        nonlocal fetch_result
        fetch_result = fetch_init_bootstrap_bars(symbol, settings=settings)
        payload = init_bootstrap_detail_payload(fetch_result)
        logger.info(
            "init kraken_fetch symbol=%s pair=%s rows=%s",
            fetch_result.symbol,
            fetch_result.kraken_rest_pair,
            fetch_result.row_count,
        )
        detail = (
            f"rows={fetch_result.row_count} pair={fetch_result.kraken_rest_pair} "
            f"wsname={fetch_result.kraken_wsname} "
            f"gran={fetch_result.granularity_seconds}s lookback_days={settings.asset_init_bootstrap_lookback_days} "
            f"meta={json.dumps(payload)}"
        )
        return "done", detail

    def do_validate() -> tuple[str, str | None]:
        nonlocal validation_result
        if fetch_result is None:
            return "failed", "internal error: kraken_fetch did not populate bootstrap result"
        validation_result = validate_and_clean_init_bootstrap_bars(
            fetch_result.dataframe,
            granularity_seconds=fetch_result.granularity_seconds,
        )
        payload = init_bootstrap_validation_detail_payload(
            validation_result,
            granularity_seconds=fetch_result.granularity_seconds,
        )
        logger.info(
            "init validate symbol=%s rows_in=%s rows_out=%s gap_intervals=%s",
            fetch_result.symbol,
            validation_result.input_rows,
            validation_result.output_rows,
            validation_result.gap_intervals,
        )
        detail = f"meta={json.dumps(payload)}"
        return "done", detail

    def do_features() -> tuple[str, str | None]:
        if validation_result is None:
            return "failed", "internal error: validate did not populate cleaned bars"
        _bars_path, _feat_path, manifest = write_init_feature_artifacts(
            symbol=symbol,
            job_id=job_id,
            cleaned_bars=validation_result.cleaned,
            settings=settings,
        )
        payload = init_features_detail_payload(manifest)
        detail = f"meta={json.dumps(payload)}"
        return "done", detail

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
    try:
        return start_asset_init_job(symbol)
    except RuntimeError:
        return None


def reset_asset_init_pipeline_for_tests() -> None:
    global _jobs, _pipeline_running
    with _jobs_lock:
        _jobs.clear()
    with _state_lock:
        _pipeline_running = False

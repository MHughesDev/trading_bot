"""
Per-asset initialization pipeline (FB-AP-006+).

Runs **one symbol at a time** in a background thread. Steps: **kraken_fetch** (FB-AP-007),
**validate** (FB-AP-008). On success the asset lifecycle transitions to
``initialized_not_active`` so it can be started for live trading.

Feature artifacts, model training, and manifest registration are intentionally
excluded — init is data-only (OHLCV from Kraken).
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
    seed_questdb = "seed_questdb"
    # kept for backwards-compat with stored job records; no longer executed
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


def _pipeline_body(job_id: str, symbol: str, lookback_days: int | None = None) -> None:
    from app.config.settings import load_settings
    from data_plane.bootstrap_bars import (
        InitBootstrapValidationResult,
        init_bootstrap_validation_detail_payload,
        validate_and_clean_init_bootstrap_bars,
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
        fetch_result = fetch_init_bootstrap_bars(symbol, settings=settings, lookback_days=lookback_days)
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

    def do_seed_questdb() -> tuple[str, str | None]:
        import asyncio

        from app.contracts.events import BarEvent
        from app.runtime.canonical_bar_watermark import write_canonical_through
        from data_plane.storage.questdb import QuestDBWriter

        if validation_result is None:
            return "failed", "internal error: validate did not populate cleaned bars"

        work = validation_result.cleaned
        if work.height == 0:
            return "skipped", "no cleaned bars to seed"

        bar_sec = fetch_result.granularity_seconds if fetch_result else int(settings.market_data_bar_interval_seconds)
        sym = symbol.strip()

        async def _insert_all() -> tuple[int, datetime | None]:
            from datetime import UTC
            qdb = QuestDBWriter(
                host=settings.questdb_host,
                port=settings.questdb_port,
                user=settings.questdb_user,
                password=settings.questdb_password,
            )
            await qdb.connect()
            inserted = 0
            max_ts: datetime | None = None
            try:
                for row in work.iter_rows(named=True):
                    ts = row.get("timestamp") or row.get("ts")
                    if not isinstance(ts, datetime):
                        continue
                    ts_utc = ts.astimezone(UTC) if ts.tzinfo else ts.replace(tzinfo=UTC)
                    bar = BarEvent(
                        timestamp=ts_utc,
                        symbol=sym,
                        open=float(row["open"]),
                        high=float(row["high"]),
                        low=float(row["low"]),
                        close=float(row["close"]),
                        volume=float(row["volume"]),
                        interval_seconds=bar_sec,
                        source="kraken_init",
                        schema_version=1,
                    )
                    await qdb.insert_bar(bar)
                    inserted += 1
                    if max_ts is None or ts_utc > max_ts:
                        max_ts = ts_utc
            finally:
                await qdb.aclose()
            return inserted, max_ts

        try:
            inserted, max_ts = asyncio.run(_insert_all())
        except Exception as exc:
            logger.warning("init seed_questdb failed symbol=%s — QuestDB may not be running: %s", sym, exc)
            return "skipped", f"QuestDB unavailable: {exc}"

        if max_ts is not None:
            try:
                write_canonical_through(sym, canonical_through_ts=max_ts, interval_seconds=bar_sec)
            except Exception as exc:
                logger.warning("init seed_questdb watermark write failed symbol=%s: %s", sym, exc)

        logger.info("init seed_questdb symbol=%s inserted=%s through=%s", sym, inserted, max_ts)
        return "done", f"inserted={inserted} through={max_ts.isoformat() if max_ts else None}"

    _run_step(job_id, InitPipelineStep.kraken_fetch, do_fetch)
    _run_step(job_id, InitPipelineStep.validate, do_validate)
    _run_step(job_id, InitPipelineStep.seed_questdb, do_seed_questdb)


def _run_pipeline(job_id: str, symbol: str, user_id: int | None = None, lookback_days: int | None = None) -> None:
    from app.runtime.tenant_context import run_with_user_id

    def _body() -> None:
        _pipeline_body(job_id, symbol, lookback_days=lookback_days)

    _update_job(job_id, status="running", started_at=_utc_now_iso())
    try:
        run_with_user_id(user_id, _body)
        _update_job(job_id, status="succeeded", finished_at=_utc_now_iso(), error=None)
        try:
            from app.runtime.asset_lifecycle_state import set_initialized_not_active

            set_initialized_not_active(symbol)
        except Exception:
            logger.exception(
                "asset init succeeded but lifecycle state not updated symbol=%s", symbol
            )
    except Exception as exc:
        logger.exception("asset init pipeline failed job_id=%s symbol=%s", job_id, symbol)
        _update_job(
            job_id,
            status="failed",
            finished_at=_utc_now_iso(),
            error=str(exc),
        )


def start_asset_init_job(symbol: str, user_id: int | None = None, lookback_days: int | None = None) -> str:
    global _pipeline_running
    with _state_lock:
        if _pipeline_running:
            raise RuntimeError("init pipeline already running for another symbol")
        _pipeline_running = True

    job_id = _new_job(symbol)

    def _target() -> None:
        global _pipeline_running
        try:
            _run_pipeline(job_id, symbol, user_id=user_id, lookback_days=lookback_days)
        finally:
            with _state_lock:
                _pipeline_running = False

    threading.Thread(target=_target, name=f"asset-init-{symbol.strip()}", daemon=True).start()
    return job_id


def try_start_asset_init_job(symbol: str, user_id: int | None = None, lookback_days: int | None = None) -> str | None:
    try:
        return start_asset_init_job(symbol, user_id=user_id, lookback_days=lookback_days)
    except RuntimeError:
        return None


def reset_asset_init_pipeline_for_tests() -> None:
    global _jobs, _pipeline_running
    with _jobs_lock:
        _jobs.clear()
    with _state_lock:
        _pipeline_running = False

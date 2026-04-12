"""FastAPI control plane — /status, /routes, /params, /system/mode, /flatten, /models."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Annotated, Any, Literal

from contextlib import asynccontextmanager

from fastapi import Depends, FastAPI, HTTPException, Query
from fastapi import status as http_status
from fastapi.responses import PlainTextResponse
from fastapi.security import APIKeyHeader
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

from observability.forecaster_metrics import MODEL_VERSION_INFO

from app.config.model_artifacts import model_artifact_contract
from app.config.settings import AppSettings, load_settings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.asset_lifecycle import AssetLifecycleState
from app.runtime.asset_execution_mode import (
    delete_mode_override,
    list_mode_overrides,
    to_api_dict as asset_execution_mode_to_api_dict,
    write_mode_override,
)
from app.runtime.asset_lifecycle_state import (
    delete_lifecycle_state,
    effective_lifecycle_state,
    lifecycle_overview,
    state_dir as asset_lifecycle_state_dir,
    transition_start,
    transition_stop,
)
from app.runtime.asset_model_registry import (
    delete_manifest,
    list_symbols as list_asset_manifest_symbols,
    load_manifest,
    registry_dir,
    save_manifest,
)
from app.runtime import asset_execution_mode as asset_execution_mode_mod
from execution.adapter_registry import supported_adapters_for_settings
from control_plane.chart_bars import query_canonical_bars_for_chart
from control_plane.preflight import preflight_report
from app.contracts.risk import SystemMode
from app.runtime.mode_manager import ModeManager
from app.runtime.state_manager import StateManager
from app.runtime.system_power import get_power, legacy_system_power_enabled, set_power
from control_plane.execution_profile import (
    apply_intent_to_config_files,
    profile_payload,
    write_pending_intent,
)
from control_plane.microservice_health import probe_microservices_health
from execution.flatten_stop import flatten_symbol_position_sync
from execution.pnl_summary import compute_pnl_series, compute_pnl_summary
from execution.portfolio_positions import fetch_portfolio_positions
from execution.service import ExecutionService
from execution.trade_markers import iter_markers, marker_to_api_dict
from orchestration.app_scheduler import (
    scheduler_status,
    start_app_background_scheduler,
    stop_app_background_scheduler,
)
from orchestration.asset_init_pipeline import get_job as get_init_job, try_start_asset_init_job

settings = load_settings()
state = StateManager()
modes = ModeManager(state)


@asynccontextmanager
async def _lifespan(app: FastAPI):
    """FB-AP-035: register background schedulers only while this process runs."""
    start_app_background_scheduler(settings)
    yield
    stop_app_background_scheduler()


app = FastAPI(title="Trading Bot Control Plane", version="0.1.0", lifespan=_lifespan)

_api_key_header = APIKeyHeader(name="X-API-Key", auto_error=False)


def production_preflight_payload(s: AppSettings) -> dict[str, Any]:
    """Signing + venue credentials vs execution mode (IL-105 / FB-SPEC-08 complement)."""
    secret_val = (
        s.risk_signing_secret.get_secret_value().strip() if s.risk_signing_secret else ""
    )
    signing_configured = bool(secret_val)
    unsigned_allowed = s.allow_unsigned_execution
    alpaca_ok = bool(
        s.alpaca_api_key
        and s.alpaca_api_secret
        and s.alpaca_api_key.get_secret_value().strip()
        and s.alpaca_api_secret.get_secret_value().strip()
    )
    coinbase_ok = bool(
        s.coinbase_api_key
        and s.coinbase_api_secret
        and s.coinbase_api_key.get_secret_value().strip()
        and s.coinbase_api_secret.get_secret_value().strip()
    )
    venue_credentials_ok = alpaca_ok if s.execution_mode == "paper" else coinbase_ok
    ok = (
        signing_configured
        and not unsigned_allowed
        and venue_credentials_ok
    )
    return {
        "ok": ok,
        "signing_secret_configured": signing_configured,
        "unsigned_execution_allowed": unsigned_allowed,
        "venue_credentials_configured": venue_credentials_ok,
        "execution_mode": s.execution_mode,
    }


def require_mutate_key(x_api_key: Annotated[str | None, Depends(_api_key_header)]) -> None:
    """Mutating endpoints require NM_CONTROL_PLANE_API_KEY when set."""
    expected = (
        settings.control_plane_api_key.get_secret_value()
        if settings.control_plane_api_key
        else None
    )
    if not expected:
        return
    if x_api_key != expected:
        raise HTTPException(status_code=http_status.HTTP_401_UNAUTHORIZED, detail="Invalid or missing API key")


@app.get("/status")
def get_status() -> dict[str, Any]:
    return {
        "execution_mode": settings.execution_mode,
        "market_data_provider": settings.market_data_provider,
        "symbols": settings.market_data_symbols,
        "mode": modes.get_mode().value,
        "system_power": get_power(),
        "system_power_legacy_enabled": legacy_system_power_enabled(),
        "preflight": preflight_report(settings),
        "production_preflight": production_preflight_payload(settings),
        "model_artifacts": model_artifact_contract(settings),
        "execution_profile": profile_payload(settings.execution_mode),
        "execution_adapters": supported_adapters_for_settings(settings),
        "asset_model_registry": {
            "registry_dir": str(registry_dir()),
            "initialized_symbols": list_asset_manifest_symbols(),
        },
        "asset_lifecycle": {
            "state_dir": str(asset_lifecycle_state_dir()),
            "states": lifecycle_overview(),
        },
        "asset_execution_mode": {
            "default_execution_mode": settings.execution_mode,
            "sidecar_dir": str(asset_execution_mode_mod.mode_dir()),
            "overrides": list_mode_overrides(),
        },
        "app_scheduler": scheduler_status(),
    }


@app.get("/system/power")
def get_system_power() -> dict[str, Any]:
    """Legacy global power (FB-AP-039: disabled by default — always ``on`` unless ``NM_SYSTEM_POWER_LEGACY_ENABLED``)."""
    return {
        "power": get_power(),
        "legacy_enabled": legacy_system_power_enabled(),
    }


@app.get("/system/execution-profile")
def get_execution_profile() -> dict[str, Any]:
    """Active vs pending ``NM_EXECUTION_MODE``; pending is set by POST until processes restart."""
    return profile_payload(settings.execution_mode)


@app.post("/system/execution-profile")
def post_execution_profile(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Record operator intent (paper/live). Optionally patch ``app/config/default.yaml`` and ``.env`` — **restart** API + live processes to load."""
    raw = str(body.get("execution_mode", "")).strip().lower()
    if raw not in ("paper", "live"):
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="execution_mode must be paper or live",
        )
    intent: Any = raw
    apply_files = bool(body.get("apply_to_config_files", True))
    files_updated: dict[str, bool] = {}
    if apply_files:
        files_updated = apply_intent_to_config_files(intent)
    write_pending_intent(intent, settings.execution_mode)
    out: dict[str, Any] = {
        **profile_payload(settings.execution_mode),
        "config_files_updated": files_updated,
        "note": "Restart control plane, live_service / live_service_app, and power_supervisor (or close and re-run run.bat) so all processes load the new mode.",
    }
    return out


@app.post("/system/power")
def post_system_power(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, str]:
    """Legacy global power (removed when ``NM_SYSTEM_POWER_LEGACY_ENABLED=false``)."""
    if not legacy_system_power_enabled():
        raise HTTPException(
            status_code=http_status.HTTP_410_GONE,
            detail="Global system power is disabled (FB-AP-039). Use per-asset Stop and process lifecycle; "
            "set NM_SYSTEM_POWER_LEGACY_ENABLED=true only if you need the legacy switch.",
        )
    raw = str(body.get("power", body.get("state", "on"))).strip().lower()
    p = "off" if raw in ("off", "false", "0") else "on"
    return {"power": set_power(p)}


@app.get("/portfolio/positions")
async def get_portfolio_positions() -> dict[str, Any]:
    """Open positions from the configured execution adapter (paper Alpaca / live Coinbase / stub)."""
    return await fetch_portfolio_positions(settings)


@app.get("/pnl/summary")
async def get_pnl_summary(
    range_key: Annotated[
        Literal["hour", "day", "month", "year", "all"],
        Query(
            alias="range",
            description="Rolling window for realized P&L from local ledger (hour/day/month/year) or all time",
        ),
    ] = "day",
) -> dict[str, Any]:
    """Aggregate realized (local JSONL ledger) + unrealized (open positions) P&L. See docs/PNL_LEDGER.MD."""
    return await compute_pnl_summary(settings, range_key)


@app.get("/pnl/series")
def get_pnl_series(
    range_key: Annotated[
        Literal["hour", "day", "month", "year", "all"],
        Query(
            alias="range",
            description="Same rolling windows as /pnl/summary",
        ),
    ] = "day",
    bucket_seconds: Annotated[
        int,
        Query(ge=60, le=86_400, description="Bucket width for ledger aggregation (seconds)"),
    ] = 3600,
) -> dict[str, Any]:
    """Cumulative realized P&L time series from the local ledger (FB-AP-026 dashboard chart)."""
    return compute_pnl_series(range_key, bucket_seconds=bucket_seconds)


@app.get("/microservices/health")
def microservices_health() -> dict[str, Any]:
    """Best-effort probes for optional scaffold processes (see infra/docker-compose.microservices.yml)."""
    return probe_microservices_health()


@app.get("/routes")
def routes() -> dict[str, list[str]]:
    return {"routes": ["NO_TRADE", "SCALPING", "INTRADAY", "SWING"]}


@app.get("/params")
def params() -> dict[str, Any]:
    return state.get_params()


@app.post("/params")
def set_params(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    state.set_params(body)
    return state.get_params()


@app.get("/system/mode")
def get_mode() -> dict[str, str]:
    return {"mode": modes.get_mode().value}


@app.post("/system/mode")
def set_mode(
    body: dict[str, str],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, str]:
    m = SystemMode(body.get("mode", "RUNNING"))
    modes.set_mode(m)
    return {"mode": modes.get_mode().value}


@app.post("/flatten")
def flatten(_: Annotated[None, Depends(require_mutate_key)]) -> dict[str, str]:
    modes.set_mode(SystemMode.FLATTEN_ALL)
    return {"mode": modes.get_mode().value, "note": "flatten requested — execution layer must honor"}


@app.get("/models")
def models() -> dict[str, list[str]]:
    return {
        "models": [
            "forecaster_xlstm_reference",
            "policy_system_mlp_heuristic",
            "risk_engine_hmac",
        ]
    }


@app.post("/models/version")
def set_model_version(
    body: dict[str, str],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, str]:
    """Expose model version labels to Prometheus (FB-PL-PG7)."""
    component = body.get("component", "forecaster")
    version = body.get("version", "unknown")
    MODEL_VERSION_INFO.labels(component=component, version=version).set(1)
    return {"component": component, "version": version}


@app.get("/metrics", response_class=PlainTextResponse)
def metrics() -> PlainTextResponse:
    return PlainTextResponse(generate_latest(), media_type=CONTENT_TYPE_LATEST)


@app.get("/assets/models")
def list_asset_model_manifests() -> dict[str, Any]:
    """List symbols with a persisted per-asset model manifest (FB-AP-002)."""
    return {
        "registry_dir": str(registry_dir()),
        "symbols": list_asset_manifest_symbols(),
    }


@app.get("/assets/models/{symbol}")
def get_asset_model_manifest(symbol: str) -> dict[str, Any]:
    """Load manifest for ``symbol``; 404 if missing."""
    m = load_manifest(symbol)
    if m is None:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="no manifest")
    return m.model_dump(mode="json")


@app.put("/assets/models/{symbol}")
def put_asset_model_manifest(
    symbol: str,
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Create or replace manifest; path ``symbol`` must match body ``canonical_symbol`` (FB-AP-001)."""
    m = AssetModelManifest.model_validate(body)
    if m.canonical_symbol.strip() != symbol.strip():
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="canonical_symbol must match path symbol",
        )
    path = save_manifest(m)
    return {"ok": True, "path": str(path), "manifest": m.model_dump(mode="json")}


@app.delete("/assets/models/{symbol}")
def remove_asset_model_manifest(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Remove manifest file for ``symbol`` if present."""
    ok = delete_manifest(symbol)
    if not ok:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="no manifest")
    delete_lifecycle_state(symbol)
    return {"ok": True, "symbol": symbol.strip()}


@app.post("/assets/init/{symbol}")
def post_asset_init(symbol: str) -> dict[str, Any]:
    """
    Start per-asset initialization (FB-AP-006): Kraken REST bootstrap, validate, enrich features
    (FB-AP-009 writes Parquet under ``data/asset_init``). One global runner — 409 if busy.
    """
    sym = symbol.strip()
    job_id = try_start_asset_init_job(sym)
    if job_id is None:
        raise HTTPException(
            status_code=http_status.HTTP_409_CONFLICT,
            detail="asset init pipeline already running",
        )
    return {"job_id": job_id, "symbol": sym}


@app.get("/assets/init/jobs/{job_id}")
def get_asset_init_job(job_id: str) -> dict[str, Any]:
    """Poll init job status and per-step detail (FB-AP-006)."""
    j = get_init_job(job_id)
    if j is None:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="unknown job_id")
    return j


@app.get("/assets/lifecycle/{symbol}")
def get_asset_lifecycle(symbol: str) -> dict[str, Any]:
    """Per-asset lifecycle state for UI buttons: Initialize / Start / Stop (FB-AP-005)."""
    sym = symbol.strip()
    return {
        "symbol": sym,
        "lifecycle_state": effective_lifecycle_state(sym).value,
    }


@app.get("/assets/execution-mode/{symbol}")
def get_asset_execution_mode(symbol: str) -> dict[str, Any]:
    """Per-symbol paper/live routing for orders (FB-AP-030); falls back to default when unset."""
    sym = symbol.strip()
    return asset_execution_mode_to_api_dict(sym, settings)


@app.put("/assets/execution-mode/{symbol}")
def put_asset_execution_mode(
    symbol: str,
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Persist paper vs live for this symbol (overrides ``NM_EXECUTION_MODE`` for its orders)."""
    sym = symbol.strip()
    mode = body.get("execution_mode")
    if mode not in ("paper", "live"):
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="execution_mode must be 'paper' or 'live'",
        )
    path = write_mode_override(sym, mode)
    return {
        "ok": True,
        "symbol": sym,
        "execution_mode": mode,
        "path": str(path),
    }


@app.delete("/assets/execution-mode/{symbol}")
def delete_asset_execution_mode(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Remove per-symbol override; routing uses application default again."""
    sym = symbol.strip()
    ok = delete_mode_override(sym)
    if not ok:
        raise HTTPException(
            status_code=http_status.HTTP_404_NOT_FOUND,
            detail="no execution_mode sidecar for symbol",
        )
    return {"ok": True, "symbol": sym}


@app.post("/assets/lifecycle/{symbol}/start")
def post_asset_lifecycle_start(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """Start watch: ``initialized_not_active`` → ``active`` (requires manifest)."""
    sym = symbol.strip()
    try:
        path = transition_start(sym)
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail=str(exc),
        ) from exc
    return {
        "ok": True,
        "symbol": sym,
        "lifecycle_state": AssetLifecycleState.active.value,
        "path": str(path),
    }


@app.get("/assets/chart/bars")
async def get_asset_chart_bars(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    start: Annotated[datetime, Query(description="Range start (UTC ISO-8601)")],
    end: Annotated[datetime, Query(description="Range end (UTC ISO-8601)")],
    interval_seconds: Annotated[
        int | None,
        Query(
            description="Bar width in seconds; default = NM_MARKET_DATA_BAR_INTERVAL_SECONDS",
        ),
    ] = None,
    limit: Annotated[int, Query(ge=1, le=50_000, description="Max rows returned")] = 5000,
) -> dict[str, Any]:
    """Symbol-scoped OHLCV from QuestDB ``canonical_bars`` for chart use (FB-AP-024)."""
    try:
        return await query_canonical_bars_for_chart(
            settings,
            symbol=symbol,
            start=start,
            end=end,
            interval_seconds=interval_seconds,
            limit=limit,
        )
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(exc),
        ) from exc


@app.get("/assets/chart/trade-markers")
def get_asset_chart_trade_markers(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    start: Annotated[datetime, Query(description="Range start (UTC ISO-8601)")],
    end: Annotated[datetime, Query(description="Range end (UTC ISO-8601)")],
    limit: Annotated[int, Query(ge=1, le=10_000, description="Max markers returned")] = 2000,
) -> dict[str, Any]:
    """Symbol-scoped buy/sell markers from append-only ``data/trade_markers.jsonl`` (FB-AP-025)."""
    sym = symbol.strip()
    s = start.replace(tzinfo=UTC) if start.tzinfo is None else start.astimezone(UTC)
    e = end.replace(tzinfo=UTC) if end.tzinfo is None else end.astimezone(UTC)
    if s >= e:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="start must be before end",
        )
    rows = iter_markers(symbol=sym, start=s, end=e)
    lim = min(int(limit), 10_000)
    if len(rows) > lim:
        rows = rows[:lim]
    return {
        "symbol": sym,
        "start": s.isoformat(),
        "end": e.isoformat(),
        "limit": lim,
        "count": len(rows),
        "source": "trade_markers_jsonl",
        "markers": [marker_to_api_dict(m) for m in rows],
    }


@app.post("/assets/lifecycle/{symbol}/stop")
def post_asset_lifecycle_stop(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_key)],
) -> dict[str, Any]:
    """
    Stop watch: flatten open venue position for this symbol (market close), then
    ``active`` → ``initialized_not_active`` (FB-AP-032).
    """
    sym = symbol.strip()
    cur = effective_lifecycle_state(sym)
    if cur != AssetLifecycleState.active:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail="not active",
        )
    exec_svc = ExecutionService(settings)
    flatten_report = flatten_symbol_position_sync(settings, sym, execution_service=exec_svc)
    if not flatten_report.get("lifecycle_continue", True):
        raise HTTPException(
            status_code=http_status.HTTP_502_BAD_GATEWAY,
            detail={
                "message": "flatten failed or could not verify position — lifecycle left active",
                "flatten": flatten_report,
            },
        )
    try:
        path = transition_stop(sym)
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail=str(exc),
        ) from exc
    return {
        "ok": True,
        "symbol": sym,
        "lifecycle_state": AssetLifecycleState.initialized_not_active.value,
        "path": str(path),
        "flatten": flatten_report,
    }

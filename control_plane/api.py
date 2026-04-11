"""FastAPI control plane — /status, /routes, /params, /system/mode, /flatten, /models."""

from __future__ import annotations

from typing import Annotated, Any

from fastapi import Depends, FastAPI, HTTPException
from fastapi import status as http_status
from fastapi.responses import PlainTextResponse
from fastapi.security import APIKeyHeader
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

from observability.forecaster_metrics import MODEL_VERSION_INFO

from app.config.model_artifacts import model_artifact_contract
from app.config.settings import AppSettings, load_settings
from control_plane.preflight import preflight_report
from app.contracts.risk import SystemMode
from app.runtime.mode_manager import ModeManager
from app.runtime.state_manager import StateManager
from app.runtime.system_power import get_power, set_power
from control_plane.execution_profile import (
    apply_intent_to_config_files,
    profile_payload,
    write_pending_intent,
)
from control_plane.microservice_health import probe_microservices_health

settings = load_settings()
state = StateManager()
modes = ModeManager(state)

app = FastAPI(title="NautilusMonster Control Plane", version="0.1.0")

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
        "preflight": preflight_report(settings),
        "production_preflight": production_preflight_payload(settings),
        "model_artifacts": model_artifact_contract(settings),
        "execution_profile": profile_payload(settings.execution_mode),
    }


@app.get("/system/power")
def get_system_power() -> dict[str, str]:
    """Global ON/OFF: OFF stops inference, trading, and offline training (see app/runtime/system_power.py)."""
    return {"power": get_power()}


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
    """Set power to ``on`` or ``off``. Persists to data/system_power.json."""
    raw = str(body.get("power", body.get("state", "on"))).strip().lower()
    p = "off" if raw in ("off", "false", "0") else "on"
    return {"power": set_power(p)}


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

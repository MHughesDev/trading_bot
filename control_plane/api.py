"""FastAPI control plane — /status, /routes, /params, /system/mode, /flatten, /models."""

from __future__ import annotations

from typing import Annotated, Any

from fastapi import Depends, FastAPI, HTTPException
from fastapi import status as http_status
from fastapi.responses import PlainTextResponse
from fastapi.security import APIKeyHeader
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

from observability.forecaster_metrics import MODEL_VERSION_INFO

from app.config.settings import AppSettings, load_settings
from control_plane.preflight import preflight_report
from app.contracts.risk import SystemMode
from app.runtime.mode_manager import ModeManager
from app.runtime.state_manager import StateManager

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
        "preflight": preflight_report(settings),
        "production_preflight": production_preflight_payload(settings),
    }


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

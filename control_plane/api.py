"""FastAPI control plane — /status, /routes, /params, /system/mode, /flatten, /models."""

from __future__ import annotations

from typing import Any

from fastapi import FastAPI
from fastapi.responses import PlainTextResponse
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

from app.config.settings import load_settings
from app.contracts.risk import SystemMode
from app.runtime.mode_manager import ModeManager
from app.runtime.state_manager import StateManager

settings = load_settings()
state = StateManager()
modes = ModeManager(state)

app = FastAPI(title="NautilusMonster Control Plane", version="0.1.0")


@app.get("/status")
def status() -> dict[str, Any]:
    return {
        "execution_mode": settings.execution_mode,
        "market_data_provider": settings.market_data_provider,
        "symbols": settings.market_data_symbols,
        "mode": modes.get_mode().value,
    }


@app.get("/routes")
def routes() -> dict[str, list[str]]:
    return {"routes": ["NO_TRADE", "SCALPING", "INTRADAY", "SWING"]}


@app.get("/params")
def params() -> dict[str, Any]:
    return state.get_params()


@app.post("/params")
def set_params(body: dict[str, Any]) -> dict[str, Any]:
    state.set_params(body)
    return state.get_params()


@app.get("/system/mode")
def get_mode() -> dict[str, str]:
    return {"mode": modes.get_mode().value}


@app.post("/system/mode")
def set_mode(body: dict[str, str]) -> dict[str, str]:
    m = SystemMode(body.get("mode", "RUNNING"))
    modes.set_mode(m)
    return {"mode": modes.get_mode().value}


@app.post("/flatten")
def flatten() -> dict[str, str]:
    modes.set_mode(SystemMode.FLATTEN_ALL)
    return {"mode": modes.get_mode().value, "note": "flatten requested — execution layer must honor"}


@app.get("/models")
def models() -> dict[str, list[str]]:
    return {"models": ["gaussian_hmm_regime", "tft_surrogate_ridge", "deterministic_router"]}


@app.get("/metrics", response_class=PlainTextResponse)
def metrics() -> PlainTextResponse:
    return PlainTextResponse(generate_latest(), media_type=CONTENT_TYPE_LATEST)

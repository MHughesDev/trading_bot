from __future__ import annotations

from contextlib import asynccontextmanager
from typing import Any

from fastapi import FastAPI, HTTPException, Response
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest
from pydantic import BaseModel

from app.config.settings import load_settings
from app.contracts.common import ExecutionMode, SystemMode
from control_plane.runtime_app import build_runtime_app

runtime_app = build_runtime_app()


class ModeUpdateRequest(BaseModel):
    mode: SystemMode


class ExecutionModeUpdateRequest(BaseModel):
    mode: ExecutionMode


@asynccontextmanager
async def lifespan(_: FastAPI):
    await runtime_app.start()
    try:
        yield
    finally:
        await runtime_app.stop()


app = FastAPI(
    title="NautilusMonster Control Plane",
    version="0.1.0",
    lifespan=lifespan,
)


@app.get("/status")
async def status() -> dict[str, Any]:
    return runtime_app.runtime.get_snapshot()


@app.get("/routes")
async def routes(limit: int = 50) -> dict[str, Any]:
    return {"items": runtime_app.runtime.list_recent_routes(limit=limit)}


@app.get("/params")
async def params() -> dict[str, Any]:
    settings = load_settings()
    return settings.model_dump(mode="json")


@app.post("/system/mode")
async def set_system_mode(req: ModeUpdateRequest) -> dict[str, str]:
    runtime_app.runtime.set_system_mode(req.mode)
    return {"status": "ok", "system_mode": req.mode.value}


@app.post("/execution/mode")
async def set_execution_mode(req: ExecutionModeUpdateRequest) -> dict[str, str]:
    runtime_app.runtime.set_execution_mode(req.mode)
    return {"status": "ok", "execution_mode": req.mode.value}


@app.post("/flatten")
async def flatten() -> dict[str, str]:
    await runtime_app.runtime.flatten_all()
    return {"status": "ok", "action": "flatten_all"}


@app.get("/models")
async def models() -> dict[str, Any]:
    s = runtime_app.settings
    return {
        "regime_model": {
            "type": "GaussianHMM",
            "n_states": s.models.regime.n_states,
            "semantic_map": ["bull", "bear", "volatile", "sideways"],
        },
        "forecast_model": {
            "type": "TemporalFusionTransformerProxy",
            "horizons": s.models.forecast.horizons,
        },
        "route_selector": {"type": "DeterministicRouteSelectorV1"},
        "memory_store": {"type": "Qdrant", "collection": s.storage.qdrant.collection},
    }


@app.get("/traces")
async def traces(limit: int = 50) -> dict[str, Any]:
    return {"items": runtime_app.runtime.list_recent_traces(limit=limit)}


@app.get("/healthz")
async def healthz() -> dict[str, str]:
    snapshot = runtime_app.runtime.get_snapshot()
    if not snapshot:
        raise HTTPException(status_code=500, detail="runtime_unavailable")
    return {"status": "ok"}


@app.get("/metrics")
async def metrics() -> Response:
    return Response(content=generate_latest(), media_type=CONTENT_TYPE_LATEST)

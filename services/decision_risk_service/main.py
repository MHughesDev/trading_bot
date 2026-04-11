"""Combined decision+risk service for milestone deployment.

This service wires `features -> decision -> risk -> execution` locally and exposes
an HTTP endpoint for smoke-testing handoff behavior.
"""

from __future__ import annotations

from typing import Any

from fastapi import FastAPI

from services.pipeline_handoff import wire_phase3_handoff
from services.decision_risk_service.wiring import create_bus
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.trace import new_trace_id

app = FastAPI(title="NautilusMonster decision_risk_service", version="0.1.0")
_bus = create_bus()
_execution = wire_phase3_handoff(_bus)


@app.get("/healthz")
def healthz() -> dict[str, str]:
    return {"status": "ok", "service": "decision_risk_service"}


@app.get("/readyz")
def readyz() -> dict[str, str]:
    return {"status": "ready", "service": "decision_risk_service"}


@app.post("/simulate")
def simulate(payload: dict[str, Any]) -> dict[str, Any]:
    """Push one feature-like event through local handoff and return latest execution state."""
    feature = EventEnvelope(
        event_type="features.row.generated",
        trace_id=new_trace_id(),
        producer_service="feature_service",
        symbol=str(payload.get("symbol", "BTC/USD")),
        payload={
            "symbol": str(payload.get("symbol", "BTC/USD")),
            "direction": int(payload.get("direction", 1)),
            "size_fraction": float(payload.get("size_fraction", 0.1)),
            "route_id": str(payload.get("route_id", "SCALPING")),
        },
    )
    _bus.publish(topics.FEATURES_ROW_GENERATED_V1, feature)
    return {
        "submitted_orders": _execution.submitted_orders,
        "submitted_count": len(_execution.submitted_orders),
    }

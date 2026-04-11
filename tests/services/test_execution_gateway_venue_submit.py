"""Execution gateway submits OrderIntent when NM_EXECUTION_GATEWAY_SUBMIT=true."""

from __future__ import annotations

import pytest
from pydantic import SecretStr

pytest.importorskip("fastapi")

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState
from execution.adapters.stub import StubExecutionAdapter
from execution.service import ExecutionService
from risk_engine.engine import RiskEngine
from services.execution_gateway_service.handlers import ExecutionGatewayHandlers
from shared.messaging import topics
from shared.messaging.envelope import EventEnvelope
from shared.messaging.in_memory import InMemoryMessageBus
from shared.messaging.security import sign_payload
from shared.messaging.trace import new_trace_id


def test_gateway_submits_to_stub_adapter(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_EXECUTION_GATEWAY_SUBMIT", "true")
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "stub")
    secret = "test-secret-for-hmac"
    monkeypatch.setenv("NM_RISK_SIGNING_SECRET", secret)
    settings = AppSettings(risk_signing_secret=SecretStr(secret), allow_unsigned_execution=False)
    bus = InMemoryMessageBus()
    stub = StubExecutionAdapter(settings)
    exec_svc = ExecutionService(settings, adapter=stub)
    gateway = ExecutionGatewayHandlers(bus, settings=settings, execution_service=exec_svc)
    gateway.register()

    risk = RiskEngine(settings)
    proposal = ActionProposal(
        symbol="BTC/USD",
        route_id=RouteId.SCALPING,
        direction=1,
        size_fraction=0.25,
        stop_distance_pct=0.0,
    )
    trade, _ = risk.evaluate(
        "BTC/USD",
        proposal,
        RiskState(),
        mid_price=50_000.0,
        spread_bps=5.0,
        data_timestamp=None,
    )
    assert trade is not None
    intent = risk.to_order_intent(trade)

    oi_dict = intent.model_dump(mode="json")
    accepted = EventEnvelope(
        event_type="risk.intent.accepted",
        trace_id=new_trace_id(),
        producer_service="risk_service",
        symbol="BTC/USD",
        payload={"order_intent": oi_dict, "message_signature": sign_payload(oi_dict, secret)},
    )
    bus.publish(topics.RISK_INTENT_ACCEPTED_V1, accepted)

    assert len(stub.submitted) == 1
    assert stub.submitted[0].symbol == "BTC/USD"
    assert len(gateway.submitted_orders) == 1

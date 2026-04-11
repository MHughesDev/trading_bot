from __future__ import annotations

from shared.messaging.envelope import EventEnvelope
from shared.messaging import topics


def test_event_envelope_defaults() -> None:
    env = EventEnvelope(
        event_type="decision.proposal.created",
        trace_id="trace-1",
        producer_service="decision_service",
        payload={"route_id": "SCALPING"},
    )

    assert str(env.event_id)
    assert env.event_version == "v1"
    assert env.ts_event is not None
    assert env.ts_ingest is not None


def test_topic_constants() -> None:
    assert topics.MARKET_TICK_NORMALIZED_V1 == "market.tick.normalized.v1"
    assert topics.DECISION_PROPOSAL_CREATED_V1 == "decision.proposal.created.v1"
    assert topics.RISK_INTENT_ACCEPTED_V1 == "risk.intent.accepted.v1"

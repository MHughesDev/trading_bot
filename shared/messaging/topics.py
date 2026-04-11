"""Canonical topic names for microservice communication."""

from __future__ import annotations

MARKET_TICK_NORMALIZED_V1 = "market.tick.normalized.v1"
MARKET_BOOK_NORMALIZED_V1 = "market.book.normalized.v1"
MARKET_HEARTBEAT_V1 = "market.heartbeat.v1"

FEATURES_ROW_GENERATED_V1 = "features.row.generated.v1"

DECISION_PROPOSAL_CREATED_V1 = "decision.proposal.created.v1"
DECISION_DIAGNOSTICS_GENERATED_V1 = "decision.diagnostics.generated.v1"

RISK_INTENT_ACCEPTED_V1 = "risk.intent.accepted.v1"
RISK_INTENT_BLOCKED_V1 = "risk.intent.blocked.v1"

EXECUTION_ORDER_ACK_V1 = "execution.order.ack.v1"
EXECUTION_ORDER_FILL_V1 = "execution.order.fill.v1"
EXECUTION_POSITION_SNAPSHOT_V1 = "execution.position.snapshot.v1"
EXECUTION_ORDER_REJECTED_V1 = "execution.order.rejected.v1"

CONTROL_MODE_CHANGED_V1 = "control.mode.changed.v1"
CONTROL_PARAMS_UPDATED_V1 = "control.params.updated.v1"

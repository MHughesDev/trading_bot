"""Risk service topic handlers for Phase 3 handoff wiring."""

from __future__ import annotations

import os
from datetime import datetime
from decimal import Decimal

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState
from risk_engine.engine import RiskEngine
from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.security import sign_payload


class RiskServiceHandlers:
    """Consumes proposals and emits accepted/blocked intents."""

    def __init__(self, bus: MessageBus, settings: AppSettings | None = None) -> None:
        self._bus = bus
        self._settings = settings or load_settings()
        self._engine = RiskEngine(self._settings)
        self._signing_secret = os.getenv("NM_RISK_SIGNING_SECRET", "dev-risk-secret")

    def on_proposal(self, env: EventEnvelope) -> None:
        direction = int(env.payload.get("direction", 0))
        symbol = str(env.payload.get("symbol", env.symbol or ""))
        if direction == 0:
            blocked = EventEnvelope(
                event_type="risk.intent.blocked",
                event_version="v1",
                trace_id=env.trace_id,
                correlation_id=str(env.event_id),
                producer_service="risk_service",
                symbol=symbol or None,
                partition_key=symbol or None,
                payload={
                    "symbol": symbol,
                    "blocked_reason": "zero_direction",
                    "proposal": env.payload,
                },
            )
            self._bus.publish(topics.RISK_INTENT_BLOCKED_V1, blocked)
            return

        raw_route = str(env.payload.get("route_id", "SCALPING"))
        try:
            route_id = RouteId(raw_route)
        except ValueError:
            route_id = RouteId.SCALPING

        proposal = ActionProposal(
            symbol=symbol,
            route_id=route_id,
            direction=direction,
            size_fraction=float(env.payload.get("size_fraction", 0.1)),
            stop_distance_pct=float(env.payload.get("stop_distance_pct", 0.0)),
            order_type=str(env.payload.get("order_type", "market")),
        )
        risk = RiskState()
        mid_price = float(env.payload.get("mid_price", 50_000.0))
        spread_bps = float(env.payload.get("spread_bps", 1.0))
        data_ts_raw = env.payload.get("data_timestamp")
        data_timestamp: datetime | None = None
        if isinstance(data_ts_raw, str):
            try:
                data_timestamp = datetime.fromisoformat(data_ts_raw.replace("Z", "+00:00"))
            except ValueError:
                data_timestamp = None
        pos_raw = env.payload.get("position_signed_qty")
        position_signed_qty: Decimal | None
        if pos_raw is not None and pos_raw != "":
            position_signed_qty = Decimal(str(pos_raw))
        else:
            position_signed_qty = None

        trade, risk_out = self._engine.evaluate(
            symbol,
            proposal,
            risk,
            mid_price=mid_price,
            spread_bps=spread_bps,
            data_timestamp=data_timestamp,
            current_total_exposure_usd=float(env.payload.get("current_total_exposure_usd", 0.0)),
            feed_last_message_at=None,
            product_tradable=bool(env.payload.get("product_tradable", True)),
            position_signed_qty=position_signed_qty,
            available_cash_usd=env.payload.get("available_cash_usd"),
        )
        if trade is None:
            blocked = EventEnvelope(
                event_type="risk.intent.blocked",
                event_version="v1",
                trace_id=env.trace_id,
                correlation_id=str(env.event_id),
                producer_service="risk_service",
                symbol=symbol or None,
                partition_key=symbol or None,
                payload={
                    "symbol": symbol,
                    "blocked_reason": "risk_evaluate_blocked",
                    "proposal": env.payload,
                    "risk_snapshot": risk_out.model_dump(mode="json"),
                },
            )
            self._bus.publish(topics.RISK_INTENT_BLOCKED_V1, blocked)
            return

        intent = self._engine.to_order_intent(trade)
        oi_dict = intent.model_dump(mode="json")
        message_signature = sign_payload(oi_dict, self._signing_secret)
        accepted = EventEnvelope(
            event_type="risk.intent.accepted",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=str(env.event_id),
            producer_service="risk_service",
            symbol=symbol or None,
            partition_key=symbol or None,
            payload={
                "order_intent": oi_dict,
                "message_signature": message_signature,
            },
        )
        self._bus.publish(topics.RISK_INTENT_ACCEPTED_V1, accepted)

    def register(self) -> None:
        self._bus.subscribe(topics.DECISION_PROPOSAL_CREATED_V1, self.on_proposal)

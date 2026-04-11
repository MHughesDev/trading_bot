"""Execution gateway topic handlers for Phase 3 handoff wiring."""

from __future__ import annotations

import asyncio
import concurrent.futures
import hashlib
import json
import logging
import os
from collections import defaultdict
from typing import Any

from pydantic import ValidationError

from app.config.settings import AppSettings, load_settings
from app.contracts.orders import OrderIntent
from execution.service import ExecutionService
from shared.messaging import topics
from shared.messaging.bus import MessageBus
from shared.messaging.envelope import EventEnvelope
from shared.messaging.security import verify_payload

logger = logging.getLogger(__name__)


def _run_async(coro: Any) -> Any:
    """Run coroutine from sync handler (safe when a loop is already running)."""
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        return pool.submit(asyncio.run, coro).result()


class ExecutionGatewayHandlers:
    """Consumes accepted risk intents and emits execution events."""

    def __init__(
        self,
        bus: MessageBus,
        *,
        settings: AppSettings | None = None,
        execution_service: ExecutionService | None = None,
    ) -> None:
        self._bus = bus
        self._settings = settings or load_settings()
        self._signing_secret = os.getenv("NM_RISK_SIGNING_SECRET", "dev-risk-secret")
        self._submit_to_venue = os.getenv("NM_EXECUTION_GATEWAY_SUBMIT", "true").strip().lower() in (
            "1",
            "true",
            "yes",
        )
        self._execution = execution_service
        if self._submit_to_venue and self._execution is None:
            self._execution = ExecutionService(self._settings)

        self.submitted_orders: list[dict[str, Any]] = []
        self._seen_intent_keys: set[str] = set()
        self._position_by_symbol: dict[str, float] = defaultdict(float)

    def _intent_dedupe_key(self, intent: OrderIntent) -> str:
        cid = intent.client_order_id
        if cid:
            return f"id:{cid}"
        raw = json.dumps(intent.model_dump(mode="json"), sort_keys=True, separators=(",", ":"))
        return "h:" + hashlib.sha256(raw.encode()).hexdigest()

    def _emit_rejected(self, env: EventEnvelope, reason: str) -> None:
        rej = EventEnvelope(
            event_type="execution.order.rejected",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=str(env.event_id),
            producer_service="execution_gateway_service",
            symbol=env.symbol,
            partition_key=env.partition_key,
            payload={"reason": reason, "raw": env.payload},
        )
        self._bus.publish(topics.EXECUTION_ORDER_REJECTED_V1, rej)

    def _emit_success_events(
        self,
        env: EventEnvelope,
        *,
        intent: OrderIntent,
        intent_key: str,
        ack_payload: dict[str, Any],
    ) -> None:
        symbol = intent.symbol
        qty = float(intent.quantity)
        side = intent.side.value
        self._position_by_symbol[symbol] += qty if side == "buy" else -qty

        ack = EventEnvelope(
            event_type="execution.order.ack",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_key,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload=ack_payload,
        )
        fill = EventEnvelope(
            event_type="execution.order.fill",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_key,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "filled_qty": qty, "side": side},
        )
        pos = EventEnvelope(
            event_type="execution.position.snapshot",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_key,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "position_qty": self._position_by_symbol[symbol]},
        )
        self._bus.publish(topics.EXECUTION_ORDER_ACK_V1, ack)
        self._bus.publish(topics.EXECUTION_ORDER_FILL_V1, fill)
        self._bus.publish(topics.EXECUTION_POSITION_SNAPSHOT_V1, pos)

    def _handle_legacy_signed_dict(self, env: EventEnvelope) -> None:
        signed_intent = env.payload.get("signed_intent")
        signature = env.payload.get("risk_signature")
        if not isinstance(signed_intent, dict) or not isinstance(signature, str):
            self._emit_rejected(env, "missing_signature_payload")
            return
        if not verify_payload(signed_intent, signature, self._signing_secret):
            self._emit_rejected(env, "invalid_signature")
            return

        intent_id = str(signed_intent.get("intent_id", ""))
        if not intent_id:
            self._emit_rejected(env, "missing_intent_id")
            return
        if intent_id in self._seen_intent_keys:
            return
        self._seen_intent_keys.add(intent_id)

        self.submitted_orders.append(signed_intent)
        symbol = str(signed_intent.get("symbol", env.symbol or ""))
        qty = float(signed_intent.get("quantity", 0.0))
        side = str(signed_intent.get("side", "buy"))
        self._position_by_symbol[symbol] += qty if side == "buy" else -qty

        ack = EventEnvelope(
            event_type="execution.order.ack",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "status": "accepted", "order_ref": f"ack-{intent_id}"},
        )
        fill = EventEnvelope(
            event_type="execution.order.fill",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "filled_qty": qty, "side": side},
        )
        pos = EventEnvelope(
            event_type="execution.position.snapshot",
            event_version="v1",
            trace_id=env.trace_id,
            correlation_id=intent_id,
            producer_service="execution_gateway_service",
            symbol=symbol or None,
            partition_key=env.partition_key,
            payload={"symbol": symbol, "position_qty": self._position_by_symbol[symbol]},
        )
        self._bus.publish(topics.EXECUTION_ORDER_ACK_V1, ack)
        self._bus.publish(topics.EXECUTION_ORDER_FILL_V1, fill)
        self._bus.publish(topics.EXECUTION_POSITION_SNAPSHOT_V1, pos)

    def on_accepted_intent(self, env: EventEnvelope) -> None:
        if "order_intent" in env.payload:
            self._handle_order_intent_envelope(env)
            return
        self._handle_legacy_signed_dict(env)

    def _handle_order_intent_envelope(self, env: EventEnvelope) -> None:
        raw_oi = env.payload.get("order_intent")
        msg_sig = env.payload.get("message_signature")
        if not isinstance(raw_oi, dict) or not isinstance(msg_sig, str):
            self._emit_rejected(env, "missing_order_intent_payload")
            return
        if not verify_payload(raw_oi, msg_sig, self._signing_secret):
            self._emit_rejected(env, "invalid_message_signature")
            return
        try:
            intent = OrderIntent.model_validate(raw_oi)
        except ValidationError:
            self._emit_rejected(env, "invalid_order_intent")
            return

        intent_key = self._intent_dedupe_key(intent)
        if intent_key in self._seen_intent_keys:
            return
        self._seen_intent_keys.add(intent_key)

        if self._submit_to_venue and self._execution is not None:
            try:
                ack = _run_async(self._execution.submit_order(intent))
            except Exception as exc:  # noqa: BLE001
                logger.exception("execution gateway submit_order failed")
                self._emit_rejected(env, f"submit_failed:{exc!s}")
                return
            self.submitted_orders.append(
                {"order_intent": intent.model_dump(mode="json"), "ack": ack.model_dump(mode="json")}
            )
            ack_payload = {
                "symbol": intent.symbol,
                "status": "accepted",
                "venue_status": ack.status,
                "adapter": ack.adapter,
                "order_id": ack.order_id,
                "raw": ack.raw,
            }
            self._emit_success_events(env, intent=intent, intent_key=intent_key, ack_payload=ack_payload)
            return

        # Scaffold: no venue submit
        self.submitted_orders.append(intent.model_dump(mode="json"))
        self._emit_success_events(
            env,
            intent=intent,
            intent_key=intent_key,
            ack_payload={
                "symbol": intent.symbol,
                "status": "accepted",
                "order_ref": f"scaffold-{intent_key[:16]}",
            },
        )

    def register(self) -> None:
        self._bus.subscribe(topics.RISK_INTENT_ACCEPTED_V1, self.on_accepted_intent)

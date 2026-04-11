"""Shared message bus selection for execution gateway (matches runtime_bridge / factory)."""

from __future__ import annotations

from shared.messaging.bus import MessageBus
from shared.messaging.factory import create_message_bus


def create_execution_gateway_bus() -> MessageBus:
    """Bus for execution gateway: ``NM_MESSAGING_BACKEND`` + ``NM_REDIS_URL``."""
    return create_message_bus()

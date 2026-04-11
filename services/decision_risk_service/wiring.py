"""Dependency wiring for combined decision+risk service (milestone path)."""

from __future__ import annotations

from shared.messaging.bus import MessageBus
from shared.messaging.factory import create_message_bus


def create_bus() -> MessageBus:
    """Create configured message bus for the combined service."""
    return create_message_bus()

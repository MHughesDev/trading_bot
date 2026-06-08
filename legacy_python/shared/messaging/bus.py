"""Message bus interfaces for service-to-service communication."""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Callable
from typing import Any

from shared.messaging.envelope import EventEnvelope

MessageHandler = Callable[[EventEnvelope], Any]


class MessageBus(ABC):
    """Abstract message bus contract used by services."""

    @abstractmethod
    def publish(self, topic: str, envelope: EventEnvelope) -> None:
        """Publish a typed envelope to a topic."""

    @abstractmethod
    def subscribe(self, topic: str, handler: MessageHandler) -> None:
        """Register a handler for a topic."""

"""Entry helpers for asyncio services (ingest + control plane)."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Coroutine
from typing import Any, TypeVar

T = TypeVar("T")

logger = logging.getLogger(__name__)


def run_async(main: Coroutine[Any, Any, T]) -> T:
    """Run coroutine with sensible defaults for services."""
    try:
        return asyncio.run(main)
    except KeyboardInterrupt:
        logger.info("shutdown requested")
        raise

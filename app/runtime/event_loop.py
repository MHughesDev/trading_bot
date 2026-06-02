"""Entry helpers for asyncio services (ingest + control plane)."""

from __future__ import annotations

import asyncio
import logging
import sys
from collections.abc import Coroutine
from typing import Any, TypeVar

T = TypeVar("T")

logger = logging.getLogger(__name__)


def run_async(main: Coroutine[Any, Any, T]) -> T:
    """Run coroutine with sensible defaults for services.

    On Windows, psycopg async mode requires SelectorEventLoop; set the policy
    before asyncio.run() so the correct loop type is created.
    """
    if sys.platform == "win32":
        asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())
    try:
        return asyncio.run(main)
    except KeyboardInterrupt:
        logger.info("shutdown requested")
        raise

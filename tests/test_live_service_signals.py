"""Shutdown signal registration (IL-011 / FB-T2)."""

from __future__ import annotations

import asyncio

import pytest

from app.runtime.live_service import register_shutdown_signals


@pytest.mark.asyncio
async def test_register_shutdown_signals_no_crash() -> None:
    stop = asyncio.Event()
    register_shutdown_signals(stop)
    # Should not raise on Unix; Windows may no-op via NotImplementedError

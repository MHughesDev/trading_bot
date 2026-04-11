"""Route OrderIntent to Alpaca (paper) or Coinbase (live) per config."""

from __future__ import annotations

import os

from app.config.settings import AppSettings
from execution.adapters.alpaca_paper import AlpacaPaperExecutionAdapter
from execution.adapters.base_adapter import ExecutionAdapter
from execution.adapters.coinbase_live import CoinbaseExecutionAdapter
from execution.adapters.stub import StubExecutionAdapter


def create_execution_adapter(settings: AppSettings) -> ExecutionAdapter:
    """Single factory for venue adapters (paper Alpaca vs live Coinbase)."""
    adapter = os.getenv("NM_EXECUTION_ADAPTER", "").strip().lower()
    if adapter == "stub":
        return StubExecutionAdapter(settings)
    if settings.execution_mode == "paper":
        if settings.execution_paper_adapter.lower() != "alpaca":
            raise ValueError(
                f"Unsupported paper adapter {settings.execution_paper_adapter!r}; spec V1 uses alpaca"
            )
        return AlpacaPaperExecutionAdapter(settings)
    if settings.execution_mode == "live":
        if settings.execution_live_adapter.lower() != "coinbase":
            raise ValueError(
                f"Unsupported live adapter {settings.execution_live_adapter!r}; spec V1 uses coinbase"
            )
        return CoinbaseExecutionAdapter(settings)
    raise ValueError(f"Invalid execution_mode {settings.execution_mode!r}")


def get_execution_adapter(settings: AppSettings) -> ExecutionAdapter:
    return create_execution_adapter(settings)

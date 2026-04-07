"""Route OrderIntent to Alpaca (paper) or Coinbase (live) per config."""

from __future__ import annotations

from app.config.settings import AppSettings
from execution.adapters.alpaca_paper import AlpacaPaperExecutionAdapter
from execution.adapters.base_adapter import ExecutionAdapter
from execution.adapters.coinbase_live import CoinbaseExecutionAdapter


def get_execution_adapter(settings: AppSettings) -> ExecutionAdapter:
    if settings.execution_mode == "paper":
        return AlpacaPaperExecutionAdapter(settings)
    return CoinbaseExecutionAdapter(settings)

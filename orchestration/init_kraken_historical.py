"""
Kraken REST historical bootstrap for per-asset init (FB-AP-007).

Fetches OHLCV for the operator symbol (e.g. ``BTC-USD``) using
:func:`orchestration.real_data_bars.fetch_symbol_bars_sync`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any

import polars as pl

from app.config.settings import AppSettings, load_settings
from data_plane.ingest.kraken_symbols import kraken_pair_from_symbol, kraken_rest_pair
from orchestration.real_data_bars import fetch_symbol_bars_sync


@dataclass(frozen=True)
class InitKrakenHistoricalResult:
    """Bootstrap fetch outcome for logging and init job ``steps`` detail."""

    symbol: str
    kraken_rest_pair: str
    kraken_wsname: str
    start_utc: datetime
    end_utc: datetime
    granularity_seconds: int
    row_count: int
    dataframe: pl.DataFrame


def resolve_init_bootstrap_granularity_seconds(settings: AppSettings) -> int:
    """Bar size for init bootstrap: explicit init override, else at least 60s for Kraken OHLC."""
    g_train = max(1, int(settings.training_data_granularity_seconds))
    if settings.asset_init_bootstrap_granularity_seconds is not None:
        return max(1, int(settings.asset_init_bootstrap_granularity_seconds))
    return max(60, g_train)


def fetch_init_bootstrap_bars(
    symbol: str,
    *,
    settings: AppSettings | None = None,
) -> InitKrakenHistoricalResult:
    """
    Pull bootstrap history from Kraken REST for first train / init pipeline.

    Range: ``now - bootstrap_lookback_days`` through ``now`` (UTC).
    """
    s = settings or load_settings()
    sym = symbol.strip()
    lookback = max(1, int(s.asset_init_bootstrap_lookback_days))
    gran = resolve_init_bootstrap_granularity_seconds(s)
    end = datetime.now(UTC)
    start = end - timedelta(days=lookback)

    df = fetch_symbol_bars_sync(sym, start, end, granularity_seconds=gran)
    rest_pair = kraken_rest_pair(sym)
    ws = kraken_pair_from_symbol(sym)
    return InitKrakenHistoricalResult(
        symbol=sym,
        kraken_rest_pair=rest_pair,
        kraken_wsname=ws,
        start_utc=start,
        end_utc=end,
        granularity_seconds=gran,
        row_count=int(df.height),
        dataframe=df,
    )


def init_bootstrap_detail_payload(result: InitKrakenHistoricalResult) -> dict[str, Any]:
    """Structured summary for API job steps (JSON-serializable)."""
    return {
        "symbol": result.symbol,
        "kraken_rest_pair": result.kraken_rest_pair,
        "kraken_wsname": result.kraken_wsname,
        "start_utc": result.start_utc.isoformat(),
        "end_utc": result.end_utc.isoformat(),
        "granularity_seconds": result.granularity_seconds,
        "rows": result.row_count,
    }

"""Tests for FB-AP-007 Kraken bootstrap helper."""

from __future__ import annotations

from datetime import UTC, datetime
from unittest.mock import patch

import polars as pl

from app.config.settings import AppSettings
from orchestration.init_kraken_historical import (
    fetch_init_bootstrap_bars,
    init_bootstrap_detail_payload,
    resolve_init_bootstrap_granularity_seconds,
)


def test_resolve_granularity_explicit_override() -> None:
    s = AppSettings(
        training_data_granularity_seconds=1,
        asset_init_bootstrap_granularity_seconds=120,
    )
    assert resolve_init_bootstrap_granularity_seconds(s) == 120


def test_resolve_granularity_defaults_to_at_least_60() -> None:
    s = AppSettings(training_data_granularity_seconds=1, asset_init_bootstrap_granularity_seconds=None)
    assert resolve_init_bootstrap_granularity_seconds(s) == 60


@patch("orchestration.init_kraken_historical.fetch_symbol_bars_sync")
def test_fetch_init_bootstrap_bars_pair_metadata(mock_fetch) -> None:
    mock_fetch.return_value = pl.DataFrame(
        {
            "timestamp": [datetime(2026, 1, 1, tzinfo=UTC)],
            "open": [1.0],
            "high": [1.0],
            "low": [1.0],
            "close": [1.0],
            "volume": [0.1],
        }
    )
    s = AppSettings(
        asset_init_bootstrap_lookback_days=7,
        training_data_granularity_seconds=60,
        asset_init_bootstrap_granularity_seconds=60,
    )
    r = fetch_init_bootstrap_bars("BTC-USD", settings=s)
    assert r.symbol == "BTC-USD"
    assert r.kraken_rest_pair == "XBTUSD"
    assert r.kraken_wsname == "XBT/USD"
    assert r.row_count == 1
    mock_fetch.assert_called_once()
    call_kw = mock_fetch.call_args.kwargs
    assert call_kw["granularity_seconds"] == 60


def test_init_bootstrap_detail_payload_json_safe() -> None:
    from orchestration.init_kraken_historical import InitKrakenHistoricalResult

    r = InitKrakenHistoricalResult(
        symbol="ETH-USD",
        kraken_rest_pair="ETHUSD",
        kraken_wsname="ETH/USD",
        start_utc=datetime(2026, 1, 1, tzinfo=UTC),
        end_utc=datetime(2026, 1, 8, tzinfo=UTC),
        granularity_seconds=60,
        row_count=10,
        dataframe=pl.DataFrame(),
    )
    d = init_bootstrap_detail_payload(r)
    assert d["kraken_rest_pair"] == "ETHUSD"
    assert d["rows"] == 10

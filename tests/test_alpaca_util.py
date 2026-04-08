"""Alpaca paper: symbol mapping + safe log redaction (Issue 18)."""

from __future__ import annotations

from execution.alpaca_util import (
    from_alpaca_crypto_symbol,
    redact_secrets_for_log,
    to_alpaca_crypto_symbol,
)


def test_to_alpaca_crypto_symbol_btc_usd() -> None:
    assert to_alpaca_crypto_symbol("BTC-USD") == "BTCUSD"


def test_to_alpaca_eth_sol() -> None:
    assert to_alpaca_crypto_symbol("ETH-USD") == "ETHUSD"
    assert to_alpaca_crypto_symbol("SOL-USD") == "SOLUSD"


def test_from_alpaca_round_trip() -> None:
    assert from_alpaca_crypto_symbol("BTCUSD") == "BTC-USD"
    assert from_alpaca_crypto_symbol("ETHUSD") == "ETH-USD"


def test_from_alpaca_already_hyphenated() -> None:
    assert from_alpaca_crypto_symbol("BTC-USD") == "BTC-USD"


def test_redact_bearer_and_key_like_strings() -> None:
    s = "Bearer sk-live-abc123 api_key: supersecret token=xyz"
    out = redact_secrets_for_log(s)
    assert "sk-live" not in out
    assert "supersecret" not in out
    assert "<redacted>" in out

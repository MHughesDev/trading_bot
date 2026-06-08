"""Alpaca paper helpers: symbol mapping and safe logging (no secrets in log lines)."""

from __future__ import annotations

import re

# Alpaca crypto symbols are like BTCUSD; Coinbase product IDs are BTC-USD.
_PAIR_RE = re.compile(r"^([A-Z]{2,10})(USD|USDT|USDC|EUR|GBP)$")


def to_alpaca_crypto_symbol(product_id: str) -> str:
    """Map BTC-USD → BTCUSD for Alpaca crypto routing."""
    return product_id.replace("-", "")


def from_alpaca_crypto_symbol(alpaca_symbol: str) -> str:
    """Best-effort map BTCUSD → BTC-USD for reconciliation with Coinbase product IDs."""
    s = (alpaca_symbol or "").strip().upper()
    if "-" in s:
        return s
    m = _PAIR_RE.match(s)
    if m:
        return f"{m.group(1)}-{m.group(2)}"
    return s


def redact_secrets_for_log(text: str) -> str:
    """Redact obvious secret patterns from error strings before logging."""
    if not text:
        return text
    out = text
    out = re.sub(r"(?i)Bearer\s+\S+", "Bearer <redacted>", out)
    out = re.sub(
        r"(?i)(api[_-]?key|secret|token|password)\s*[:=]\s*[^\s,;]+",
        lambda m: f"{m.group(1)}=<redacted>",
        out,
    )
    return out


def safe_exc_message(exc: BaseException) -> str:
    """One-line exception text safe for INFO/WARNING logs."""
    msg = f"{type(exc).__name__}: {exc}"
    return redact_secrets_for_log(msg)[:500]

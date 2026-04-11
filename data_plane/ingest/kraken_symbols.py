"""Map human-readable symbols (BTC-USD) to Kraken pair names (XBT/USD)."""

from __future__ import annotations

# Kraken uses XBT for Bitcoin in pair names; quote is often ZUSD for USD.
# wsname / human (slash) — WebSocket and display
_KNOWN_WS = {
    "BTC-USD": "XBT/USD",
    "ETH-USD": "ETH/USD",
    "SOL-USD": "SOL/USD",
    "BTC-USDT": "XBT/USDT",
    "ETH-USDT": "ETH/USDT",
}
# REST ``pair`` parameter often uses altname without slash (e.g. XBTUSD)
_KNOWN_REST = {
    "BTC-USD": "XBTUSD",
    "ETH-USD": "ETHUSD",
    "SOL-USD": "SOLUSD",
    "BTC-USDT": "XBTUSDT",
    "ETH-USDT": "ETHUSDT",
}


def kraken_pair_from_symbol(symbol: str) -> str:
    """
    Convert config symbol like ``BTC-USD`` to Kraken **wsname** pair ``XBT/USD``.
    """
    s = symbol.strip().upper()
    if s in _KNOWN_WS:
        return _KNOWN_WS[s]
    if "-" in s:
        base, quote = s.split("-", 1)
        if base == "BTC":
            base = "XBT"
        return f"{base}/{quote}"
    return s.replace("-", "/")


def kraken_rest_pair(symbol: str) -> str:
    """
    REST ``pair`` query value (e.g. ``XBTUSD``) for ``/OHLC``, ``/Trades``.
    """
    s = symbol.strip().upper()
    if s in _KNOWN_REST:
        return _KNOWN_REST[s]
    if "-" in s:
        base, quote = s.split("-", 1)
        if base == "BTC":
            base = "XBT"
        return f"{base}{quote}"
    return s.replace("-", "").replace("/", "")


def kraken_ws_pair(symbol: str) -> str:
    """Pair for Kraken WebSocket v1 ``pair`` field — use wsname ``XBT/USD``."""
    return kraken_pair_from_symbol(symbol)

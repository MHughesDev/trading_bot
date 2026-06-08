"""FB-UX-014 watchlist helpers."""

from __future__ import annotations

from control_plane.watchlist import (
    MAX_WATCHLIST_SIZE,
    add_watchlist_symbol,
    get_watchlist_symbols,
    remove_watchlist_symbol,
)


class _FakeSession:
    def __init__(self) -> None:
        self._d: dict[str, object] = {}

    def get(self, k: str, default=None):
        return self._d.get(k, default)

    def __setitem__(self, k: str, v: object) -> None:
        self._d[k] = v

    def __getitem__(self, k: str) -> object:
        return self._d[k]


def test_add_remove_roundtrip() -> None:
    ss = _FakeSession()
    ok, _ = add_watchlist_symbol(ss, "  btc-usd  ")
    assert ok
    assert get_watchlist_symbols(ss) == ["BTC-USD"]
    remove_watchlist_symbol(ss, "BTC-USD")
    assert get_watchlist_symbols(ss) == []


def test_max_size() -> None:
    ss = _FakeSession()
    for i in range(MAX_WATCHLIST_SIZE):
        ok, _ = add_watchlist_symbol(ss, f"X{i}-USD")
        assert ok
    ok, _ = add_watchlist_symbol(ss, "Z99-USD")
    assert not ok
    assert len(get_watchlist_symbols(ss)) == MAX_WATCHLIST_SIZE

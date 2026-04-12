"""Session watchlist / favorites (FB-UX-014) — quick navigation to Asset page."""

from __future__ import annotations

from typing import Any

from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display

WATCHLIST_SESSION_KEY = "watchlist_symbols"
MAX_WATCHLIST_SIZE = 24


def _as_list(raw: Any) -> list[str]:
    if not isinstance(raw, list):
        return []
    out: list[str] = []
    for x in raw:
        s = normalize_symbol(str(x))
        if validate_symbol_display(s) and s not in out:
            out.append(s)
        if len(out) >= MAX_WATCHLIST_SIZE:
            break
    return out


def get_watchlist_symbols(session_state: Any) -> list[str]:
    return _as_list(session_state.get(WATCHLIST_SESSION_KEY))


def set_watchlist_symbols(session_state: Any, symbols: list[str]) -> None:
    session_state[WATCHLIST_SESSION_KEY] = _as_list(symbols)


def add_watchlist_symbol(session_state: Any, raw: str) -> tuple[bool, str]:
    """
    Add ``raw`` to the watchlist. Returns ``(ok, message)`` — ok False if invalid or full.
    """
    sym = normalize_symbol(raw)
    if not validate_symbol_display(sym):
        return False, "Invalid symbol."
    cur = get_watchlist_symbols(session_state)
    if sym in cur:
        return True, "Already pinned."
    if len(cur) >= MAX_WATCHLIST_SIZE:
        return False, f"Watchlist full ({MAX_WATCHLIST_SIZE} symbols)."
    set_watchlist_symbols(session_state, cur + [sym])
    return True, "Pinned."


def remove_watchlist_symbol(session_state: Any, raw: str) -> None:
    sym = normalize_symbol(raw)
    cur = [s for s in get_watchlist_symbols(session_state) if s != sym]
    set_watchlist_symbols(session_state, cur)


def is_pinned(session_state: Any, raw: str) -> bool:
    sym = normalize_symbol(raw)
    return sym in get_watchlist_symbols(session_state)

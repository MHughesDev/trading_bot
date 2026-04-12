"""Symbol helpers for Asset Page (FB-AP-027) — no Streamlit import."""

from __future__ import annotations


def normalize_symbol(raw: str) -> str:
    return raw.strip().upper()


def validate_symbol_display(sym: str) -> bool:
    """Loose validation for UI; API remains authoritative."""
    if not sym or len(sym) > 64:
        return False
    return "/" not in sym and "\\" not in sym and not sym.startswith(".")

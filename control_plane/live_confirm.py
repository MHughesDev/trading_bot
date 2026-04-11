"""Guardrails before applying **live** execution profile in Streamlit (FB-DASH-06-01)."""

from __future__ import annotations

import os

LIVE_CONFIRM_PHRASE = "LIVE"


def live_confirm_env_enabled() -> bool:
    """Default on; set ``NM_STREAMLIT_LIVE_CONFIRM=false`` to skip (dev/automation)."""
    v = os.getenv("NM_STREAMLIT_LIVE_CONFIRM", "true").strip().lower()
    return v not in ("0", "false", "no", "off")


def requires_live_confirmation(mode_choice: str, active_execution_mode: str) -> bool:
    """
    Require checkbox + typed phrase when switching **to** live from a non-live active mode.

    Re-applying while already live does not require the phrase again.
    """
    if not live_confirm_env_enabled():
        return False
    if mode_choice.strip().lower() != "live":
        return False
    return active_execution_mode.strip().lower() != "live"


def live_apply_allowed(
    mode_choice: str,
    active_execution_mode: str,
    *,
    acknowledge_risk: bool,
    typed_phrase: str,
) -> bool:
    """Return True if Apply should call the API for this mode transition."""
    if mode_choice.strip().lower() != "live":
        return True
    if not requires_live_confirmation(mode_choice, active_execution_mode):
        return True
    if not acknowledge_risk:
        return False
    return typed_phrase.strip().upper() == LIVE_CONFIRM_PHRASE

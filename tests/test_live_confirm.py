"""Streamlit live execution confirmation helpers."""

from __future__ import annotations

import pytest

from control_plane import live_confirm as lc


def test_requires_live_confirmation(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("NM_STREAMLIT_LIVE_CONFIRM", raising=False)
    assert lc.requires_live_confirmation("live", "paper") is True
    assert lc.requires_live_confirmation("paper", "paper") is False
    assert lc.requires_live_confirmation("live", "live") is False


def test_requires_live_confirmation_disabled(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_STREAMLIT_LIVE_CONFIRM", "false")
    assert lc.requires_live_confirmation("live", "paper") is False


def test_live_apply_allowed() -> None:
    assert lc.live_apply_allowed("paper", "live", acknowledge_risk=False, typed_phrase="") is True
    assert (
        lc.live_apply_allowed(
            "live",
            "paper",
            acknowledge_risk=True,
            typed_phrase=lc.LIVE_CONFIRM_PHRASE,
        )
        is True
    )
    assert (
        lc.live_apply_allowed(
            "live",
            "paper",
            acknowledge_risk=False,
            typed_phrase=lc.LIVE_CONFIRM_PHRASE,
        )
        is False
    )
    assert (
        lc.live_apply_allowed(
            "live",
            "paper",
            acknowledge_risk=True,
            typed_phrase="wrong",
        )
        is False
    )


def test_live_confirm_phrase_constant() -> None:
    assert lc.LIVE_CONFIRM_PHRASE == "LIVE"

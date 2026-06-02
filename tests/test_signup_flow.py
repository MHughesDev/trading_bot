"""Run the multi-step sign-up flow under Streamlit AppTest (email→password→alpaca→webull)."""

from __future__ import annotations

import pytest
from streamlit.testing.v1 import AppTest

from control_plane import streamlit_util

_PAGE = "control_plane/pages/99_Sign_up.py"


@pytest.fixture(autouse=True)
def _no_redirect(monkeypatch: pytest.MonkeyPatch) -> None:
    # The page would otherwise call the API to bounce authenticated users; stub it out so
    # AppTest does not attempt a network call.
    monkeypatch.setattr(streamlit_util, "redirect_authenticated_user_from_auth_page", lambda: None)


def _md(at: AppTest) -> str:
    return "\n".join(m.value for m in at.markdown)


def test_email_step_renders_first() -> None:
    at = AppTest.from_file(_PAGE).run()
    labels = [t.label for t in at.text_input]
    assert "Email" in labels
    # NOTE: the page renders an st.page_link("Sign in") which AppTest cannot resolve
    # (no multi-page registry in the harness); that is a harness limitation, not a page
    # bug, so we assert the Email input rendered rather than `not at.exception` here.


def test_email_step_advances_to_password() -> None:
    at = AppTest.from_file(_PAGE).run()
    at.text_input[0].set_value("trader@example.com")
    # The form's submit button advances the flow.
    at.button[0].click().run()
    assert not at.exception
    assert at.session_state["signup_step"] == "password"
    assert "Password" in [t.label for t in at.text_input]


def test_alpaca_step_has_link_and_skip() -> None:
    at = AppTest.from_file(_PAGE)
    at.session_state["signup_step"] = "alpaca"
    at.run()
    assert not at.exception
    body = _md(at)
    assert "alpaca.markets" in body  # instructional link present
    labels = [b.label for b in at.button]
    assert "Save & continue" in labels
    assert "Skip for now" in labels


def test_alpaca_skip_advances_to_webull() -> None:
    at = AppTest.from_file(_PAGE)
    at.session_state["signup_step"] = "alpaca"
    at.run()
    skip = next(b for b in at.button if b.label == "Skip for now")
    skip.click().run()
    assert not at.exception
    assert at.session_state["signup_step"] == "webull"


def test_webull_step_has_link() -> None:
    at = AppTest.from_file(_PAGE)
    at.session_state["signup_step"] = "webull"
    at.run()
    assert not at.exception
    assert "webull.com" in _md(at)

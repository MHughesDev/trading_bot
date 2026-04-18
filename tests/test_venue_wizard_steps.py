"""FB-UX-017 wizard flow source contract checks."""

from __future__ import annotations

from pathlib import Path


SRC = Path("control_plane/pages/98_Setup_API_keys.py").read_text(encoding="utf-8")


def test_wizard_step_states_and_progress_labels_present() -> None:
    assert '"alpaca", "coinbase", "done"' in SRC
    assert "Step 1 of 2" in SRC
    assert "Step 2 of 2" in SRC


def test_wizard_has_next_and_skip_paths_for_both_steps() -> None:
    assert 'button("Next", type="primary"' in SRC
    assert "Skip (paper trading will be unavailable)" in SRC
    assert "Skip (live trading will be unavailable)" in SRC
    assert 'st.session_state["venue_setup_step"] = "coinbase"' in SRC
    assert 'st.session_state["venue_setup_step"] = "done"' in SRC
    assert "st.rerun()" in SRC


def test_done_step_redirects_to_home() -> None:
    assert 'st.switch_page("Home.py")' in SRC

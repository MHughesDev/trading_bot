"""scheduler_panel nightly message helper (FB-UX-013)."""

from __future__ import annotations

from control_plane.scheduler_panel import _nightly_finish_message


def test_message_skipped_no_manifest() -> None:
    s = _nightly_finish_message({"skipped": True, "reason": "no_manifest_symbols"}, error=None)
    assert "skipped" in s.lower()


def test_message_rl_skipped_per_asset() -> None:
    rep = {
        "per_asset": True,
        "reports": {
            "BTC-USD": {"rl_skipped": {"reason": "no_trade_markers_in_lookback", "symbol": "BTC-USD"}},
        },
    }
    s = _nightly_finish_message(rep, error=None)
    assert "RL skipped" in s or "trades" in s


def test_message_error() -> None:
    s = _nightly_finish_message({}, error="boom")
    assert "failed" in s.lower()

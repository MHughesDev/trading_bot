"""scheduler_panel helpers (FB-UX-012)."""

from __future__ import annotations

from control_plane.scheduler_panel import _fmt_report_brief


def test_fmt_report_brief_none() -> None:
    assert _fmt_report_brief(None) == "—"


def test_fmt_report_brief_truncates() -> None:
    long_obj = {"x": "y" * 2000}
    s = _fmt_report_brief(long_obj, max_chars=50)
    assert "truncated" in s or len(s) <= 60

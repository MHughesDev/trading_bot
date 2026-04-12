"""health_strip helpers (FB-UX-008)."""

from __future__ import annotations

from control_plane.health_strip import _fmt_issues


def test_fmt_issues_empty() -> None:
    assert _fmt_issues([]) == ""


def test_fmt_issues_truncates() -> None:
    s = _fmt_issues(["a", "b", "c", "d"], max_items=2)
    assert "…" in s or "+2 more" in s

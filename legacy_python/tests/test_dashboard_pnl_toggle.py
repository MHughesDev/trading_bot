"""FB-UX-020 dashboard mode/range helpers."""

from __future__ import annotations

from control_plane.pnl_panel import _bucket_seconds_for_range, _series_path


def test_series_path_includes_mode_query_param() -> None:
    p = _series_path("day", "live", 3600)
    assert p == "/pnl/series?range=day&bucket_seconds=3600&mode=live"


def test_bucket_seconds_mapping() -> None:
    assert _bucket_seconds_for_range("hour") == 300
    assert _bucket_seconds_for_range("day") == 3600
    assert _bucket_seconds_for_range("month") == 86_400

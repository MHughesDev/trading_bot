"""Tier-1 fingerprints (SPEC 5.2, checklist 3.1/3.2, ADR-P3-01)."""
from __future__ import annotations

import math, pathlib, sys
import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import fingerprint as fp  # noqa: E402

HOUR = 3_600_000_000_000


def series(n=600, seed=1, drift=0.0, vol=0.01, continuous=True, signed=False):
    r = np.random.default_rng(seed)
    rets = r.normal(drift, vol, n)
    close = 100 * np.exp(np.cumsum(rets))
    high = close * (1 + np.abs(r.normal(0, vol / 2, n)))
    low = close * (1 - np.abs(r.normal(0, vol / 2, n)))
    open_ = np.concatenate([[100.0], close[:-1]])
    return fp.Bars(
        ts_ns=np.arange(n, dtype=np.int64) * HOUR,
        open=open_, high=high, low=low, close=close,
        volume=np.full(n, 1_000.0),
        signed_volume=(r.normal(0, 100, n) if signed else None),
        continuous_venue=continuous,
    )


def test_a_fingerprint_measures_what_it_can_and_names_what_it_cannot():
    f = fp.compute(series())
    assert f.values, "a 600-bar series is measurable"
    # Absence is always explained, never a zero.
    for name, reason in f.absent.items():
        assert reason, name
    assert set(f.values) & set(f.absent) == set(), "a dimension is measured or absent, not both"


def test_a_continuous_venue_has_no_overnight_rather_than_a_quiet_one():
    f = fp.compute(series(continuous=True))
    assert "overnight_share" not in f.values
    assert f.absent["overnight_share"] == "continuous_venue"
    # A venue that closes does have one.
    g = fp.compute(series(continuous=False))
    assert 0.0 <= g.values["overnight_share"] <= 1.0


def test_kyle_lambda_is_absent_without_signed_volume():
    """Most venues on free data publish no trade side. An unsigned-volume
    regression estimates something else and wears the same name."""
    assert "kyle_lambda" in fp.compute(series(signed=False)).absent
    assert "kyle_lambda" in fp.compute(series(signed=True, n=400)).values


def test_seasonal_deflation_removes_an_hour_of_week_shape():
    """Crypto's hour-of-week volatility shape otherwise dominates the
    fingerprint, which then encodes what time the window started."""
    r = np.random.default_rng(7)
    n = 2000
    ts = np.arange(n, dtype=np.int64) * HOUR
    hours = (ts // HOUR) % fp.HOURS_PER_WEEK
    # Four times the volatility in one stretch of the week.
    scale = np.where(hours < 24, 4.0, 1.0)
    rets = r.normal(0, 0.01, n) * scale
    deflated = fp.deflate_seasonality(ts, rets[1:])
    loud = np.std(rets[1:][hours[1:] < 24])
    quiet = np.std(rets[1:][hours[1:] >= 24])
    assert loud / quiet > 3, "the fixture really is seasonal"
    d_loud = np.std(deflated[hours[1:] < 24])
    d_quiet = np.std(deflated[hours[1:] >= 24])
    assert abs(d_loud / d_quiet - 1) < 0.3, f"deflation should flatten it: {d_loud/d_quiet}"


def test_the_variance_ratio_separates_trending_from_mean_reverting():
    r = np.random.default_rng(3)
    walk = r.normal(0, 0.01, 3000)
    assert fp.variance_ratio(walk, 5) == pytest.approx(1.0, abs=0.25)

    reverting = np.empty(3000)
    reverting[0] = 0.0
    for i in range(1, 3000):
        reverting[i] = -0.6 * reverting[i - 1] + r.normal(0, 0.01)
    assert fp.variance_ratio(reverting, 5) < 0.7


def test_the_jump_share_is_larger_when_returns_actually_jump():
    r = np.random.default_rng(5)
    smooth = r.normal(0, 0.01, 1000)
    jumpy = smooth.copy()
    jumpy[::100] += 0.2
    assert fp.bipower_jump_share(jumpy) > fp.bipower_jump_share(smooth)


def test_edge_returns_a_spread_or_nothing_never_a_number_from_four_bars():
    assert fp.edge_spread(series(n=8)) is None
    s = fp.edge_spread(series(n=500))
    assert s is None or s >= 0.0


def test_a_hurst_exponent_near_a_half_is_a_random_walk():
    r = np.random.default_rng(11)
    h = fp.hurst_exponent(r.normal(0, 0.01, 2000))
    assert h == pytest.approx(0.5, abs=0.15), h


def test_the_manifest_carries_what_was_never_measured():
    """A consumer that treats a missing dimension as zero is inventing a
    measurement; the only way to stop that is to hand it the absence list."""
    f = fp.compute(series(continuous=True))
    m = fp.as_manifest(f, instrument_id="BTC-USD", venue_id=1, window_spec="21d@1h")
    assert m["info_class"] == "market_public"
    assert "overnight_share" in m["absent"]
    assert "overnight_share" not in m["dimensions"]


def test_a_series_too_short_to_measure_says_so_rather_than_returning_zeros():
    f = fp.compute(fp.Bars(
        ts_ns=np.array([0], dtype=np.int64),
        open=np.array([1.0]), high=np.array([1.0]),
        low=np.array([1.0]), close=np.array([1.0]), volume=np.array([1.0]),
    ))
    assert f.values == {}
    assert "all" in f.absent

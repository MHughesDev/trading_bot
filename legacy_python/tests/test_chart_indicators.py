"""Pure-pandas technical-indicator registry: every study computes; key ones value-checked."""

from __future__ import annotations

import numpy as np
import pytest

pd = pytest.importorskip("pandas")  # charts package requires the optional [dashboard] extra

from charts._helpers import compute_indicator_frames  # noqa: E402
from charts.indicators import (  # noqa: E402
    REGISTRY,
    available_indicators,
    indicator_label_to_key,
    keys_from_labels,
    overlay_keys,
    sub_pane_keys,
)


def _frame(n: int = 260) -> pd.DataFrame:
    rng = np.random.default_rng(7)
    t = np.arange(n)
    close = 100.0 + 8.0 * np.sin(t / 11.0) + np.cumsum(rng.normal(0, 0.2, size=n))
    high = close + np.abs(rng.normal(0, 0.5, size=n))
    low = close - np.abs(rng.normal(0, 0.5, size=n))
    open_ = np.concatenate([[close[0]], close[:-1]])
    volume = 1_000_000 + rng.integers(0, 500_000, size=n)
    return pd.DataFrame(
        {
            "time": pd.date_range("2026-01-01", periods=n, freq="1min", tz="UTC"),
            "open": open_,
            "high": np.maximum.reduce([high, open_, close]),
            "low": np.minimum.reduce([low, open_, close]),
            "close": close,
            "volume": volume.astype(float),
        }
    )


def test_registry_split_and_menu() -> None:
    assert "sma" in overlay_keys() and "bollinger_bands" in overlay_keys()
    assert "rsi" in sub_pane_keys() and "macd" in sub_pane_keys()
    assert overlay_keys().isdisjoint(sub_pane_keys())
    specs = available_indicators()
    assert len(specs) == len(REGISTRY) >= 25  # comprehensive overlay + oscillator set


@pytest.mark.parametrize("key", sorted(REGISTRY))
def test_every_indicator_computes_finite(key: str) -> None:
    frame = _frame()
    out = compute_indicator_frames(frame, key)
    assert out, f"{key} produced no series"
    for series_name, sf in out.items():
        assert list(sf.columns) == ["time", series_name]
        # At least the warmed-up tail must be finite (not an all-NaN series).
        assert sf[series_name].notna().any(), f"{key}:{series_name} is all-NaN"
        assert np.isfinite(sf[series_name].to_numpy()).all()


def test_unknown_indicator_raises() -> None:
    with pytest.raises(ValueError):
        compute_indicator_frames(_frame(), "definitely_not_an_indicator")


def test_label_key_roundtrip_for_multiselect() -> None:
    mapping = indicator_label_to_key()
    # Every label maps to a registered key.
    assert all(key in REGISTRY for key in mapping.values())
    # Selecting labels yields the matching keys; unknown labels are ignored.
    keys = keys_from_labels(["EMA (Exponential MA)", "Bollinger Bands", "Not An Indicator"])
    assert keys == ["ema", "bollinger_bands"]


def test_value_bounds_and_relationships() -> None:
    frame = _frame()

    # SMA equals the rolling mean of close.
    sma = compute_indicator_frames(frame, "sma", length=10)["SMA 10"]
    expected = frame["close"].rolling(10).mean().dropna().reset_index(drop=True)
    assert np.allclose(sma["SMA 10"].to_numpy(), expected.to_numpy())

    # Bollinger: Upper >= Basis >= Lower everywhere.
    bb = compute_indicator_frames(frame, "bollinger_bands", length=20)
    merged = (
        bb["BB Upper 20"].merge(bb["BB Basis 20"], on="time").merge(bb["BB Lower 20"], on="time")
    )
    assert (merged["BB Upper 20"] >= merged["BB Basis 20"] - 1e-9).all()
    assert (merged["BB Basis 20"] >= merged["BB Lower 20"] - 1e-9).all()

    # Bounded oscillators.
    rsi = compute_indicator_frames(frame, "rsi")["RSI 14"]["RSI 14"]
    assert rsi.between(0, 100).all()
    wr = compute_indicator_frames(frame, "williams_r")["Williams %R 14"]["Williams %R 14"]
    assert wr.between(-100, 0).all()
    k = compute_indicator_frames(frame, "stochastic")["Stoch %K"]["Stoch %K"]
    assert k.between(-0.001, 100.001).all()


def test_donchian_contains_price() -> None:
    frame = _frame()
    dc = compute_indicator_frames(frame, "donchian_channels", length=20)
    upper = dc["DC Upper 20"].set_index("time")["DC Upper 20"]
    lower = dc["DC Lower 20"].set_index("time")["DC Lower 20"]
    close = frame.set_index("time")["close"].reindex(upper.index)
    assert (close <= upper + 1e-9).all() and (close >= lower - 1e-9).all()

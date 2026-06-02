"""TradingChart: Heikin-Ashi transform, chart-type switch, and registry-driven sub-panes."""

from __future__ import annotations

import numpy as np
import pytest

pd = pytest.importorskip("pandas")

from charts._helpers import heikin_ashi  # noqa: E402
from charts.trading_chart import TradingChart  # noqa: E402


def _frame(n: int = 120) -> pd.DataFrame:
    rng = np.random.default_rng(3)
    close = 100.0 + np.cumsum(rng.normal(0, 0.3, size=n))
    open_ = np.concatenate([[close[0]], close[:-1]])
    high = np.maximum(open_, close) + np.abs(rng.normal(0, 0.4, size=n))
    low = np.minimum(open_, close) - np.abs(rng.normal(0, 0.4, size=n))
    return pd.DataFrame(
        {
            "time": pd.date_range("2026-01-01", periods=n, freq="1min", tz="UTC"),
            "open": open_,
            "high": high,
            "low": low,
            "close": close,
            "volume": np.full(n, 1e6),
        }
    )


class _StubSource:
    def __init__(self, frame: pd.DataFrame) -> None:
        self._frame = frame

    def get_ohlcv(self, *, symbol: str, timeframe: str) -> pd.DataFrame:
        return self._frame


def test_heikin_ashi_transform() -> None:
    f = _frame(40)
    ha = heikin_ashi(f)
    # HA close is the OHLC average.
    expected_close = (f["open"] + f["high"] + f["low"] + f["close"]) / 4
    assert np.allclose(ha["close"].to_numpy(), expected_close.to_numpy())
    # HA high/low bracket the HA open/close, and high >= low everywhere.
    assert (ha["high"] >= ha[["open", "close"]].max(axis=1) - 1e-9).all()
    assert (ha["low"] <= ha[["open", "close"]].min(axis=1) + 1e-9).all()
    assert (ha["high"] >= ha["low"]).all()


def test_render_candles_with_overlays_and_oscillators() -> None:
    chart = TradingChart("BTC-USD", "1D", data_source=_StubSource(_frame()))
    chart.add_indicator("sma", length=20).add_indicator("supertrend")
    chart.add_indicator("stochastic").add_indicator("rsi")
    html = chart.render_html()
    assert "addCandlestickSeries" in html
    assert '"SMA 20"' in html and '"SuperTrend"' in html  # overlays drawn
    assert 'stochastic-pane' in html and 'rsi-pane' in html  # oscillator sub-panes
    assert "LineStyle.Dashed" in html  # reference levels (RSI 70/30, Stoch 80/20)


def test_render_line_type() -> None:
    chart = TradingChart("BTC-USD", "1D", chart_type="line", data_source=_StubSource(_frame()))
    html = chart.render_html()
    assert 'chartType = "line"' in html
    assert "addLineSeries" in html


def test_render_heikin_ashi_type() -> None:
    chart = TradingChart("BTC-USD", "1D", chart_type="heikin_ashi", data_source=_StubSource(_frame()))
    html = chart.render_html()
    assert 'chartType = "heikin_ashi"' in html
    assert "addCandlestickSeries" in html  # HA still renders as candles, just transformed data


def test_unknown_chart_type_falls_back_to_candles() -> None:
    chart = TradingChart("BTC-USD", "1D", chart_type="bogus", data_source=_StubSource(_frame()))
    assert chart.chart_type == "candles"

"""Technical-indicator registry for the reusable trading chart.

Pure-pandas implementations (no talib / pandas_ta required) so every study computes in any
environment and is unit-testable. Each indicator declares its ``pane`` (price ``overlay`` vs a
separate ``sub`` oscillator pane), category, default params, and the named series it outputs.
``charts._helpers.compute_indicator_frames`` dispatches through :data:`REGISTRY`, and the
Streamlit asset page builds its multiselect from :func:`available_indicators`.

Series naming is stable (e.g. ``"SMA 20"``, ``"BB Upper 20"``, ``"MACD"``) so chart colours and
existing renderers keep working.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field

import numpy as np
import pandas as pd

# A compute function takes the OHLCV frame + params and returns {series_name: series}.
ComputeFn = Callable[[pd.DataFrame, dict], dict[str, "pd.Series"]]


@dataclass(frozen=True)
class IndicatorSpec:
    key: str
    label: str
    pane: str  # "overlay" (drawn over price) | "sub" (separate oscillator pane)
    category: str
    compute: ComputeFn
    default_params: dict = field(default_factory=dict)
    reference_levels: tuple[float, ...] = ()  # horizontal guide lines in a sub-pane


REGISTRY: dict[str, IndicatorSpec] = {}


def _register(spec: IndicatorSpec) -> IndicatorSpec:
    REGISTRY[spec.key] = spec
    return spec


# --------------------------------------------------------------------------- math helpers
def _sma(s: pd.Series, n: int) -> pd.Series:
    return s.rolling(n).mean()


def _ema(s: pd.Series, n: int) -> pd.Series:
    return s.ewm(span=n, adjust=False).mean()


def _rma(s: pd.Series, n: int) -> pd.Series:
    """Wilder's smoothing (used by RSI/ATR/ADX)."""
    return s.ewm(alpha=1.0 / n, adjust=False).mean()


def _wma(s: pd.Series, n: int) -> pd.Series:
    weights = np.arange(1, n + 1, dtype=float)
    return s.rolling(n).apply(lambda x: float(np.dot(x, weights) / weights.sum()), raw=True)


def _true_range(high: pd.Series, low: pd.Series, close: pd.Series) -> pd.Series:
    prev_close = close.shift(1)
    return pd.concat(
        [(high - low), (high - prev_close).abs(), (low - prev_close).abs()], axis=1
    ).max(axis=1)


def _atr(high: pd.Series, low: pd.Series, close: pd.Series, n: int) -> pd.Series:
    return _rma(_true_range(high, low, close), n)


def _typical_price(frame: pd.DataFrame) -> pd.Series:
    return (frame["high"] + frame["low"] + frame["close"]) / 3.0


# --------------------------------------------------------------------------- moving averages
def _c_sma(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    return {f"SMA {n}": _sma(f["close"], n)}


def _c_ema(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    return {f"EMA {n}": _ema(f["close"], n)}


def _c_wma(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    return {f"WMA {n}": _wma(f["close"], n)}


def _c_vwma(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    pv = (f["close"] * f["volume"]).rolling(n).sum()
    vv = f["volume"].rolling(n).sum().replace(0, np.nan)
    return {f"VWMA {n}": pv / vv}


def _c_hma(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    half = max(1, n // 2)
    sqrt_n = max(1, int(round(np.sqrt(n))))
    raw = 2 * _wma(f["close"], half) - _wma(f["close"], n)
    return {f"HMA {n}": _wma(raw, sqrt_n)}


def _c_dema(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    e1 = _ema(f["close"], n)
    return {f"DEMA {n}": 2 * e1 - _ema(e1, n)}


def _c_tema(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    e1 = _ema(f["close"], n)
    e2 = _ema(e1, n)
    e3 = _ema(e2, n)
    return {f"TEMA {n}": 3 * e1 - 3 * e2 + e3}


# --------------------------------------------------------------------------- bands / channels
def _c_bollinger(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    k = float(p.get("std_dev", 2.0))
    mid = _sma(f["close"], n)
    sd = f["close"].rolling(n).std()
    return {
        f"BB Upper {n}": mid + k * sd,
        f"BB Basis {n}": mid,
        f"BB Lower {n}": mid - k * sd,
    }


def _c_keltner(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    mult = float(p.get("mult", 2.0))
    mid = _ema(f["close"], n)
    atr = _atr(f["high"], f["low"], f["close"], n)
    return {
        f"KC Upper {n}": mid + mult * atr,
        f"KC Basis {n}": mid,
        f"KC Lower {n}": mid - mult * atr,
    }


def _c_donchian(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    upper = f["high"].rolling(n).max()
    lower = f["low"].rolling(n).min()
    return {f"DC Upper {n}": upper, f"DC Basis {n}": (upper + lower) / 2.0, f"DC Lower {n}": lower}


def _c_envelopes(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    pct = float(p.get("percent", 2.5)) / 100.0
    mid = _sma(f["close"], n)
    return {f"ENV Upper {n}": mid * (1 + pct), f"ENV Basis {n}": mid, f"ENV Lower {n}": mid * (1 - pct)}


def _c_vwap(f: pd.DataFrame, p: dict) -> dict:
    tp = _typical_price(f)
    cum_v = f["volume"].replace(0, np.nan).cumsum()
    return {"VWAP": (tp * f["volume"]).cumsum() / cum_v}


# --------------------------------------------------------------------------- trend overlays
def _c_ichimoku(f: pd.DataFrame, p: dict) -> dict:
    conv_n = int(p.get("conversion", 9))
    base_n = int(p.get("base", 26))
    span_n = int(p.get("span_b", 52))
    high, low = f["high"], f["low"]
    conv = (high.rolling(conv_n).max() + low.rolling(conv_n).min()) / 2.0
    base = (high.rolling(base_n).max() + low.rolling(base_n).min()) / 2.0
    span_a = (conv + base) / 2.0
    span_b = (high.rolling(span_n).max() + low.rolling(span_n).min()) / 2.0
    # Note: classic Ichimoku shifts the cloud forward `base_n`; omitted here since the embedded
    # chart's time axis is not extended into the future. Conversion/Base are the actionable lines.
    return {
        "Ichimoku Conversion": conv,
        "Ichimoku Base": base,
        "Ichimoku Span A": span_a,
        "Ichimoku Span B": span_b,
    }


def _c_parabolic_sar(f: pd.DataFrame, p: dict) -> dict:
    af_step = float(p.get("step", 0.02))
    af_max = float(p.get("max", 0.2))
    high = f["high"].to_numpy(dtype=float)
    low = f["low"].to_numpy(dtype=float)
    n = len(high)
    sar = np.full(n, np.nan)
    if n < 2:
        return {"PSAR": pd.Series(sar, index=f.index)}
    up = True  # start assuming an uptrend
    af = af_step
    ep = high[0]
    sar[0] = low[0]
    for i in range(1, n):
        prev = sar[i - 1]
        cur = prev + af * (ep - prev)
        if up:
            cur = min(cur, low[i - 1], low[max(0, i - 2)])
            if low[i] < cur:  # flip to downtrend
                up = False
                cur = ep
                ep = low[i]
                af = af_step
            elif high[i] > ep:
                ep = high[i]
                af = min(af + af_step, af_max)
        else:
            cur = max(cur, high[i - 1], high[max(0, i - 2)])
            if high[i] > cur:  # flip to uptrend
                up = True
                cur = ep
                ep = high[i]
                af = af_step
            elif low[i] < ep:
                ep = low[i]
                af = min(af + af_step, af_max)
        sar[i] = cur
    return {"PSAR": pd.Series(sar, index=f.index)}


def _c_supertrend(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 10))
    mult = float(p.get("mult", 3.0))
    hl2 = (f["high"] + f["low"]) / 2.0
    atr = _atr(f["high"], f["low"], f["close"], n)
    upper = np.array(hl2 + mult * atr, dtype=float)  # writable copies (bands ratchet in-place)
    lower = np.array(hl2 - mult * atr, dtype=float)
    close = f["close"].to_numpy(dtype=float)
    m = len(close)
    st = np.full(m, np.nan)
    trend_up = True
    for i in range(m):
        if i == 0 or np.isnan(atr.iloc[i]):
            st[i] = lower[i]
            continue
        if trend_up:
            lower[i] = max(lower[i], lower[i - 1]) if close[i - 1] > lower[i - 1] else lower[i]
            st[i] = lower[i]
            if close[i] < lower[i]:
                trend_up = False
                st[i] = upper[i]
        else:
            upper[i] = min(upper[i], upper[i - 1]) if close[i - 1] < upper[i - 1] else upper[i]
            st[i] = upper[i]
            if close[i] > upper[i]:
                trend_up = True
                st[i] = lower[i]
    return {"SuperTrend": pd.Series(st, index=f.index)}


def _c_pivots(f: pd.DataFrame, p: dict) -> dict:
    # Previous-bar floor-trader pivots (works on any timeframe without resampling).
    ph, pl, pc = f["high"].shift(1), f["low"].shift(1), f["close"].shift(1)
    pivot = (ph + pl + pc) / 3.0
    rng = ph - pl
    return {
        "Pivot": pivot,
        "Pivot R1": 2 * pivot - pl,
        "Pivot S1": 2 * pivot - ph,
        "Pivot R2": pivot + rng,
        "Pivot S2": pivot - rng,
    }


# --------------------------------------------------------------------------- oscillators (sub)
def _c_rsi(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    delta = f["close"].diff()
    gain = _rma(delta.clip(lower=0), n)
    loss = _rma((-delta.clip(upper=0)), n)
    rs = gain / loss.replace(0, np.nan)
    return {f"RSI {n}": 100 - (100 / (1 + rs))}


def _c_macd(f: pd.DataFrame, p: dict) -> dict:
    fast = int(p.get("fast", 12))
    slow = int(p.get("slow", 26))
    sig = int(p.get("signal", 9))
    macd = _ema(f["close"], fast) - _ema(f["close"], slow)
    signal = _ema(macd, sig)
    return {"MACD": macd, "MACD Signal": signal, "MACD Histogram": macd - signal}


def _c_stochastic(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    d = int(p.get("smooth_d", 3))
    ll = f["low"].rolling(n).min()
    hh = f["high"].rolling(n).max()
    k = 100 * (f["close"] - ll) / (hh - ll).replace(0, np.nan)
    return {"Stoch %K": k, "Stoch %D": _sma(k, d)}


def _c_stoch_rsi(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    rsi = _c_rsi(f, {"length": n})[f"RSI {n}"]
    ll = rsi.rolling(n).min()
    hh = rsi.rolling(n).max()
    k = 100 * (rsi - ll) / (hh - ll).replace(0, np.nan)
    return {"StochRSI %K": k, "StochRSI %D": _sma(k, 3)}


def _c_cci(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 20))
    tp = _typical_price(f)
    sma_tp = _sma(tp, n)
    mad = (tp - sma_tp).abs().rolling(n).mean()
    return {f"CCI {n}": (tp - sma_tp) / (0.015 * mad.replace(0, np.nan))}


def _c_williams_r(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    hh = f["high"].rolling(n).max()
    ll = f["low"].rolling(n).min()
    return {f"Williams %R {n}": -100 * (hh - f["close"]) / (hh - ll).replace(0, np.nan)}


def _c_roc(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 12))
    return {f"ROC {n}": 100 * (f["close"] / f["close"].shift(n) - 1)}


def _c_awesome(f: pd.DataFrame, p: dict) -> dict:
    median = (f["high"] + f["low"]) / 2.0
    return {"Awesome Osc": _sma(median, 5) - _sma(median, 34)}


def _c_obv(f: pd.DataFrame, p: dict) -> dict:
    direction = np.sign(f["close"].diff().fillna(0.0))
    return {"OBV": (direction * f["volume"]).cumsum()}


def _c_mfi(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    tp = _typical_price(f)
    rmf = tp * f["volume"]
    up = rmf.where(tp > tp.shift(1), 0.0)
    down = rmf.where(tp < tp.shift(1), 0.0)
    ratio = up.rolling(n).sum() / down.rolling(n).sum().replace(0, np.nan)
    return {f"MFI {n}": 100 - (100 / (1 + ratio))}


def _c_adx(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    up_move = f["high"].diff()
    down_move = -f["low"].diff()
    plus_dm = up_move.where((up_move > down_move) & (up_move > 0), 0.0)
    minus_dm = down_move.where((down_move > up_move) & (down_move > 0), 0.0)
    atr = _atr(f["high"], f["low"], f["close"], n).replace(0, np.nan)
    plus_di = 100 * _rma(plus_dm, n) / atr
    minus_di = 100 * _rma(minus_dm, n) / atr
    dx = 100 * (plus_di - minus_di).abs() / (plus_di + minus_di).replace(0, np.nan)
    return {f"ADX {n}": _rma(dx, n), "+DI": plus_di, "-DI": minus_di}


def _c_atr(f: pd.DataFrame, p: dict) -> dict:
    n = int(p.get("length", 14))
    return {f"ATR {n}": _atr(f["high"], f["low"], f["close"], n)}


def _c_volume(f: pd.DataFrame, p: dict) -> dict:
    return {"Volume": f["volume"]}


# --------------------------------------------------------------------------- registry
for _spec in (
    # price overlays
    IndicatorSpec("sma", "SMA (Simple MA)", "overlay", "Moving Average", _c_sma, {"length": 20}),
    IndicatorSpec("ema", "EMA (Exponential MA)", "overlay", "Moving Average", _c_ema, {"length": 20}),
    IndicatorSpec("wma", "WMA (Weighted MA)", "overlay", "Moving Average", _c_wma, {"length": 20}),
    IndicatorSpec("vwma", "VWMA (Volume-Weighted MA)", "overlay", "Moving Average", _c_vwma, {"length": 20}),
    IndicatorSpec("hma", "HMA (Hull MA)", "overlay", "Moving Average", _c_hma, {"length": 20}),
    IndicatorSpec("dema", "DEMA", "overlay", "Moving Average", _c_dema, {"length": 20}),
    IndicatorSpec("tema", "TEMA", "overlay", "Moving Average", _c_tema, {"length": 20}),
    IndicatorSpec("bollinger_bands", "Bollinger Bands", "overlay", "Volatility", _c_bollinger, {"length": 20, "std_dev": 2.0}),
    IndicatorSpec("keltner_channels", "Keltner Channels", "overlay", "Volatility", _c_keltner, {"length": 20, "mult": 2.0}),
    IndicatorSpec("donchian_channels", "Donchian Channels", "overlay", "Volatility", _c_donchian, {"length": 20}),
    IndicatorSpec("envelopes", "Envelopes", "overlay", "Volatility", _c_envelopes, {"length": 20, "percent": 2.5}),
    IndicatorSpec("vwap", "VWAP", "overlay", "Volume", _c_vwap),
    IndicatorSpec("ichimoku", "Ichimoku Cloud", "overlay", "Trend", _c_ichimoku, {"conversion": 9, "base": 26, "span_b": 52}),
    IndicatorSpec("parabolic_sar", "Parabolic SAR", "overlay", "Trend", _c_parabolic_sar, {"step": 0.02, "max": 0.2}),
    IndicatorSpec("supertrend", "SuperTrend", "overlay", "Trend", _c_supertrend, {"length": 10, "mult": 3.0}),
    IndicatorSpec("pivot_points", "Pivot Points", "overlay", "Trend", _c_pivots),
    # sub-pane oscillators
    IndicatorSpec("rsi", "RSI", "sub", "Momentum", _c_rsi, {"length": 14}, (70.0, 30.0)),
    IndicatorSpec("macd", "MACD", "sub", "Momentum", _c_macd, {"fast": 12, "slow": 26, "signal": 9}),
    IndicatorSpec("stochastic", "Stochastic", "sub", "Momentum", _c_stochastic, {"length": 14, "smooth_d": 3}, (80.0, 20.0)),
    IndicatorSpec("stoch_rsi", "Stochastic RSI", "sub", "Momentum", _c_stoch_rsi, {"length": 14}, (80.0, 20.0)),
    IndicatorSpec("cci", "CCI", "sub", "Momentum", _c_cci, {"length": 20}, (100.0, -100.0)),
    IndicatorSpec("williams_r", "Williams %R", "sub", "Momentum", _c_williams_r, {"length": 14}, (-20.0, -80.0)),
    IndicatorSpec("roc", "Rate of Change", "sub", "Momentum", _c_roc, {"length": 12}, (0.0,)),
    IndicatorSpec("awesome_oscillator", "Awesome Oscillator", "sub", "Momentum", _c_awesome, {}, (0.0,)),
    IndicatorSpec("obv", "On-Balance Volume", "sub", "Volume", _c_obv),
    IndicatorSpec("mfi", "Money Flow Index", "sub", "Volume", _c_mfi, {"length": 14}, (80.0, 20.0)),
    IndicatorSpec("adx", "ADX / DMI", "sub", "Trend", _c_adx, {"length": 14}, (25.0,)),
    IndicatorSpec("atr", "ATR", "sub", "Volatility", _c_atr, {"length": 14}),
    IndicatorSpec("volume", "Volume", "sub", "Volume", _c_volume),
):
    _register(_spec)


def overlay_keys() -> frozenset[str]:
    return frozenset(k for k, s in REGISTRY.items() if s.pane == "overlay")


def sub_pane_keys() -> frozenset[str]:
    return frozenset(k for k, s in REGISTRY.items() if s.pane == "sub")


def available_indicators() -> list[IndicatorSpec]:
    """Specs for UI menus, ordered by category then label."""
    return sorted(REGISTRY.values(), key=lambda s: (s.category, s.label))


def reference_levels(key: str) -> tuple[float, ...]:
    spec = REGISTRY.get(key.lower().replace(" ", "_"))
    return spec.reference_levels if spec else ()


def indicator_label_to_key() -> dict[str, str]:
    """Ordered {label: key} map for building a UI multiselect (category, label order)."""
    return {spec.label: spec.key for spec in available_indicators()}


def keys_from_labels(labels: list[str]) -> list[str]:
    """Map selected multiselect labels back to registry keys (unknown labels ignored)."""
    mapping = indicator_label_to_key()
    return [mapping[label] for label in labels if label in mapping]

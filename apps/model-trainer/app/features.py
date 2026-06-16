"""Feature and label computation for training on real OHLCV bars.

Mirrors the feature names exposed by the Rust `features` crate
(`fs_core_ohlcv_v3` and friends) so the columns a model trains on match what the
live inference path can produce. Given a per-instrument OHLCV frame, computes the
requested feature columns plus a forward-return ``label`` column.
"""

import numpy as np
import pandas as pd

# Timeframe / horizon token → minutes.
_UNIT_MINUTES = {"s": 1 / 60, "m": 1, "h": 60, "d": 1440}


def _token_to_minutes(token: str) -> float:
    token = token.strip().lower()
    unit = token[-1]
    value = float(token[:-1])
    return value * _UNIT_MINUTES.get(unit, 1)


def horizon_in_bars(label_horizon: str, timeframe: str) -> int:
    """Number of bars that make up ``label_horizon`` at ``timeframe`` resolution."""
    bars = _token_to_minutes(label_horizon) / max(_token_to_minutes(timeframe), 1e-9)
    return max(int(round(bars)), 1)


def _rsi(close: pd.Series, period: int) -> pd.Series:
    delta = close.diff()
    gain = delta.clip(lower=0.0)
    loss = -delta.clip(upper=0.0)
    avg_gain = gain.ewm(alpha=1 / period, adjust=False, min_periods=period).mean()
    avg_loss = loss.ewm(alpha=1 / period, adjust=False, min_periods=period).mean()
    rs = avg_gain / avg_loss.replace(0.0, np.nan)
    return 100.0 - (100.0 / (1.0 + rs))


def _compute_one(name: str, df: pd.DataFrame) -> pd.Series | None:
    """Compute a single named feature column from an OHLCV frame, or None."""
    close = df["close"]
    if name in ("open", "high", "low", "close", "volume"):
        return df[name]
    if name.startswith("ema_"):
        period = int(name.split("_")[1])
        return close.ewm(span=period, adjust=False).mean()
    if name.startswith("rsi_"):
        return _rsi(close, int(name.split("_")[1]))
    if name.startswith("rolling_mean_"):
        return close.rolling(int(name.split("_")[2])).mean()
    if name.startswith("rolling_std_"):
        return close.rolling(int(name.split("_")[2])).std()
    if name.startswith("returns_"):
        return close.pct_change(int(name.split("_")[1]))
    if name == "log_returns_1":
        return np.log(close / close.shift(1))
    return None


def build_training_frame(
    bars: pd.DataFrame,
    features: list[str],
    timeframe: str,
    label_horizon: str,
) -> pd.DataFrame:
    """Build a (features + ``label``) frame from one instrument's bar history.

    ``label`` is the forward simple return over ``label_horizon``. Rows with any
    NaN (warm-up period, trailing label window) are dropped.
    """
    if bars.empty:
        return pd.DataFrame(columns=[*features, "label"])

    bars = bars.sort_values("ts_ms").reset_index(drop=True)

    out = pd.DataFrame(index=bars.index)
    for name in features:
        col = _compute_one(name, bars)
        if col is not None:
            out[name] = col

    h = horizon_in_bars(label_horizon, timeframe)
    out["label"] = bars["close"].shift(-h) / bars["close"] - 1.0

    return out.dropna().reset_index(drop=True)

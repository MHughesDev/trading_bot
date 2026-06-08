"""Polars feature pipeline: returns, vol, RSI, MACD, ATR, ADX, EMA spread, VWAP distance, microstructure."""

from __future__ import annotations

import polars as pl


def _rsi(close: pl.Expr, period: int = 14) -> pl.Expr:
    delta = close.diff()
    gain = pl.when(delta > 0).then(delta).otherwise(0.0)
    loss = pl.when(delta < 0).then(-delta).otherwise(0.0)
    avg_gain = gain.ewm_mean(span=period, adjust=False)
    avg_loss = loss.ewm_mean(span=period, adjust=False)
    rs = avg_gain / (avg_loss + 1e-12)
    return 100.0 - (100.0 / (1.0 + rs))


def _ema(col: pl.Expr, span: int) -> pl.Expr:
    return col.ewm_mean(span=span, adjust=False)


def _atr(high: pl.Expr, low: pl.Expr, close: pl.Expr, period: int = 14) -> pl.Expr:
    prev_close = close.shift(1)
    tr = pl.max_horizontal(
        high - low,
        (high - prev_close).abs(),
        (low - prev_close).abs(),
    )
    return tr.ewm_mean(span=period, adjust=False)


def _macd(close: pl.Expr, fast: int = 12, slow: int = 26, signal: int = 9) -> tuple[pl.Expr, pl.Expr, pl.Expr]:
    ema_fast = _ema(close, fast)
    ema_slow = _ema(close, slow)
    macd_line = ema_fast - ema_slow
    signal_line = macd_line.ewm_mean(span=signal, adjust=False)
    hist = macd_line - signal_line
    return macd_line, signal_line, hist


def _adx(high: pl.Expr, low: pl.Expr, close: pl.Expr, period: int = 14) -> pl.Expr:
    up_move = high - high.shift(1)
    down_move = low.shift(1) - low
    plus_dm = pl.when((up_move > down_move) & (up_move > 0)).then(up_move).otherwise(0.0)
    minus_dm = pl.when((down_move > up_move) & (down_move > 0)).then(down_move).otherwise(0.0)
    prev_close = close.shift(1)
    tr = pl.max_horizontal(
        high - low,
        (high - prev_close).abs(),
        (low - prev_close).abs(),
    )
    atr = tr.ewm_mean(span=period, adjust=False)
    plus_di = 100.0 * (plus_dm.ewm_mean(span=period, adjust=False) / (atr + 1e-12))
    minus_di = 100.0 * (minus_dm.ewm_mean(span=period, adjust=False) / (atr + 1e-12))
    dx = ((plus_di - minus_di).abs() / (plus_di + minus_di + 1e-12)) * 100.0
    return dx.ewm_mean(span=period, adjust=False)


class FeaturePipeline:
    def __init__(
        self,
        return_windows: list[int] | None = None,
        volatility_windows: list[int] | None = None,
    ) -> None:
        self.return_windows = return_windows or [1, 3, 5, 15]
        self.volatility_windows = volatility_windows or [5, 15, 60]

    def enrich_bars(self, df: pl.DataFrame) -> pl.DataFrame:
        """
        Expect columns: timestamp, open, high, low, close, volume (and optionally symbol).
        Appends feature columns in place order.
        """
        if df.height == 0:
            return df
        out = df.sort("timestamp")
        close = pl.col("close")
        high = pl.col("high")
        low = pl.col("low")
        vol = pl.col("volume")

        for w in self.return_windows:
            out = out.with_columns((close.pct_change(w)).alias(f"ret_{w}"))

        for w in self.volatility_windows:
            out = out.with_columns(close.pct_change().rolling_std(window_size=w).alias(f"vol_{w}"))

        macd, signal, _hist = _macd(close)
        out = out.with_columns(
            _rsi(close).alias("rsi_14"),
            macd.alias("macd"),
            signal.alias("macd_signal"),
            _atr(high, low, close).alias("atr_14"),
            _adx(high, low, close).alias("adx_14"),
            (_ema(close, 12) - _ema(close, 26)).alias("ema_spread_12_26"),
        )

        cum_vp = (close * vol).cum_sum()
        cum_v = vol.cum_sum()
        vwap = cum_vp / (cum_v + 1e-12)
        out = out.with_columns(vwap.alias("vwap_cum"))
        out = out.with_columns(((close - pl.col("vwap_cum")) / (pl.col("vwap_cum") + 1e-12)).alias("vwap_dist"))

        return out

    def microstructure(
        self,
        spread: float | None,
        bid_sz: float,
        ask_sz: float,
        volume_delta: float,
    ) -> dict[str, float]:
        imb = (bid_sz - ask_sz) / (bid_sz + ask_sz + 1e-12)
        return {
            "spread": float(spread or 0.0),
            "book_imbalance": float(imb),
            "volume_delta": float(volume_delta),
            "liquidity_pressure": float(abs(imb)),
        }

    def sentiment_features(
        self,
        *,
        finbert_score: float = 0.0,
        news_count_per_hour: float = 0.0,
        sentiment_shock: float = 0.0,
    ) -> dict[str, float]:
        """Spec §5.3 — wire FinBERT + frequency + shocks from news pipeline."""
        return {
            "sent_finbert": float(finbert_score),
            "sent_news_freq": float(news_count_per_hour),
            "sent_shock": float(sentiment_shock),
        }

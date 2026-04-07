from __future__ import annotations

import math
from dataclasses import dataclass

import polars as pl

from app.contracts.events import BarEvent


def _safe_div(n: float, d: float) -> float:
    return n / d if d else 0.0


@dataclass(slots=True)
class FeatureBuilder:
    """
    Polars feature pipeline for market + microstructure features.

    Features in V1:
    - returns: 1/3/5/15
    - rolling volatility
    - RSI-like momentum
    - EMA spreads
    - VWAP distance
    """

    def bars_to_frame(self, bars: list[BarEvent]) -> pl.DataFrame:
        rows = [
            {
                "timestamp": b.timestamp,
                "symbol": b.symbol,
                "open": b.open,
                "high": b.high,
                "low": b.low,
                "close": b.close,
                "volume": b.volume,
            }
            for b in bars
        ]
        if not rows:
            return pl.DataFrame(
                schema={
                    "timestamp": pl.Datetime(time_zone="UTC"),
                    "symbol": pl.String,
                    "open": pl.Float64,
                    "high": pl.Float64,
                    "low": pl.Float64,
                    "close": pl.Float64,
                    "volume": pl.Float64,
                }
            )
        return pl.DataFrame(rows).sort(["symbol", "timestamp"])

    def compute(self, bars: list[BarEvent]) -> pl.DataFrame:
        df = self.bars_to_frame(bars)
        if df.height == 0:
            return df

        out = df.with_columns(
            [
                pl.col("close").log().diff().over("symbol").alias("ret_1"),
                (pl.col("close") / pl.col("close").shift(3).over("symbol") - 1).alias("ret_3"),
                (pl.col("close") / pl.col("close").shift(5).over("symbol") - 1).alias("ret_5"),
                (pl.col("close") / pl.col("close").shift(15).over("symbol") - 1).alias("ret_15"),
                pl.col("close").rolling_std(window_size=14).over("symbol").alias("vol_14"),
                pl.col("close").ewm_mean(span=12).over("symbol").alias("ema_12"),
                pl.col("close").ewm_mean(span=26).over("symbol").alias("ema_26"),
                (
                    (pl.col("close") * pl.col("volume")).cum_sum().over("symbol")
                    / pl.col("volume").cum_sum().over("symbol")
                ).alias("vwap"),
            ]
        ).with_columns(
            [
                (pl.col("ema_12") - pl.col("ema_26")).alias("ema_spread"),
                ((pl.col("close") - pl.col("vwap")) / pl.col("vwap")).alias("vwap_distance"),
            ]
        )

        # RSI approximation on close returns; kept explicit for readability.
        out = self._append_rsi(out)
        return out

    def _append_rsi(self, df: pl.DataFrame) -> pl.DataFrame:
        if df.height < 2:
            return df.with_columns(pl.lit(50.0).alias("rsi_14"))

        close = df["close"].to_list()
        gains: list[float] = [0.0]
        losses: list[float] = [0.0]
        for i in range(1, len(close)):
            ch = float(close[i]) - float(close[i - 1])
            gains.append(max(ch, 0.0))
            losses.append(max(-ch, 0.0))

        window = 14
        rsi_vals: list[float] = []
        for i in range(len(close)):
            lo = max(0, i - window + 1)
            avg_gain = sum(gains[lo : i + 1]) / max((i - lo + 1), 1)
            avg_loss = sum(losses[lo : i + 1]) / max((i - lo + 1), 1)
            rs = _safe_div(avg_gain, avg_loss)
            rsi = 100 - (100 / (1 + rs)) if avg_loss > 0 else 100.0
            if math.isnan(rsi):
                rsi = 50.0
            rsi_vals.append(rsi)

        return df.with_columns(pl.Series(name="rsi_14", values=rsi_vals))

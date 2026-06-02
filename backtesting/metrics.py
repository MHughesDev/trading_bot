"""Performance metrics for backtests: returns, Sharpe/Sortino, drawdown, trade stats.

Consumes the per-bar row dicts from :func:`backtesting.replay.replay_decisions` /
:func:`backtesting.replay.replay_multi_asset_decisions`. The equity curve is read from
``equity_mark`` (single-asset) or ``portfolio_equity_mark`` (multi-asset). Trade-level
statistics come from :mod:`backtesting.trade_ledger`.

Default annualization assumes a 24/7 (crypto) calendar derived from the bar interval;
override ``periods_per_year`` for equities or other calendars.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any

from backtesting.trade_ledger import TradeStats, build_trade_ledger, summarize_trades

__all__ = [
    "BacktestMetrics",
    "extract_equity_curve",
    "periodic_returns",
    "max_drawdown",
    "sharpe_ratio",
    "sortino_ratio",
    "annualization_factor",
    "compute_backtest_metrics",
]

_SECONDS_PER_YEAR_247 = 365.0 * 24.0 * 3600.0


def _to_float(value: Any) -> float | None:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def extract_equity_curve(rows: list[dict[str, Any]]) -> tuple[list[Any], list[float]]:
    """Return ``(timestamps, equity)`` from replay rows.

    Uses ``equity_mark`` for single-asset rows and ``portfolio_equity_mark`` for
    multi-asset rows. Rows without an equity value are skipped (e.g. when the replay
    was run without ``track_portfolio=True``).
    """
    timestamps: list[Any] = []
    equity: list[float] = []
    for row in rows:
        raw = row.get("equity_mark")
        if raw is None:
            raw = row.get("portfolio_equity_mark")
        value = _to_float(raw)
        if value is None:
            continue
        timestamps.append(row.get("timestamp"))
        equity.append(value)
    return timestamps, equity


def periodic_returns(equity: list[float]) -> list[float]:
    """Simple per-bar returns of the equity curve (length ``len(equity) - 1``)."""
    out: list[float] = []
    for prev, cur in zip(equity, equity[1:]):
        if prev == 0:
            out.append(0.0)
        else:
            out.append((cur - prev) / prev)
    return out


def max_drawdown(equity: list[float]) -> float:
    """Maximum peak-to-trough drawdown as a positive fraction (0.0 = none)."""
    peak = -math.inf
    mdd = 0.0
    for value in equity:
        if value > peak:
            peak = value
        if peak > 0:
            dd = (peak - value) / peak
            if dd > mdd:
                mdd = dd
    return mdd


def _mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def _stdev(values: list[float], *, sample: bool = True) -> float:
    n = len(values)
    if n < 2:
        return 0.0
    mu = _mean(values)
    var = sum((v - mu) ** 2 for v in values) / (n - 1 if sample else n)
    return math.sqrt(var)


def annualization_factor(bar_interval_seconds: float, *, calendar_seconds_per_year: float | None = None) -> float:
    """Number of bars per year for the given bar interval (24/7 calendar by default)."""
    spy = calendar_seconds_per_year or _SECONDS_PER_YEAR_247
    if bar_interval_seconds <= 0:
        return 0.0
    return spy / float(bar_interval_seconds)


def sharpe_ratio(
    returns: list[float], *, periods_per_year: float, risk_free_rate: float = 0.0
) -> float:
    """Annualized Sharpe ratio. ``risk_free_rate`` is an annual rate."""
    if len(returns) < 2 or periods_per_year <= 0:
        return 0.0
    rf_per_period = risk_free_rate / periods_per_year
    excess = [r - rf_per_period for r in returns]
    sd = _stdev(excess)
    if sd == 0:
        return 0.0
    return (_mean(excess) / sd) * math.sqrt(periods_per_year)


def sortino_ratio(
    returns: list[float], *, periods_per_year: float, risk_free_rate: float = 0.0
) -> float:
    """Annualized Sortino ratio (downside-deviation denominator)."""
    if len(returns) < 2 or periods_per_year <= 0:
        return 0.0
    rf_per_period = risk_free_rate / periods_per_year
    excess = [r - rf_per_period for r in returns]
    downside = [min(0.0, r) for r in excess]
    dd = math.sqrt(sum(d * d for d in downside) / len(downside)) if downside else 0.0
    if dd == 0:
        return 0.0
    return (_mean(excess) / dd) * math.sqrt(periods_per_year)


@dataclass
class BacktestMetrics:
    """Aggregate backtest performance metrics (equity-curve + trade-level)."""

    n_bars: int = 0
    initial_equity: float = 0.0
    final_equity: float = 0.0
    total_return: float = 0.0
    cumulative_pnl: float = 0.0
    cagr: float = 0.0
    annualized_volatility: float = 0.0
    sharpe_ratio: float = 0.0
    sortino_ratio: float = 0.0
    max_drawdown: float = 0.0
    calmar_ratio: float = 0.0
    periods_per_year: float = 0.0
    trade_stats: TradeStats = field(default_factory=TradeStats)

    def to_dict(self) -> dict[str, Any]:
        data = {k: v for k, v in self.__dict__.items() if k != "trade_stats"}
        data["trade_stats"] = self.trade_stats.to_dict()
        return data


def compute_backtest_metrics(
    rows: list[dict[str, Any]],
    *,
    bar_interval_seconds: float = 60.0,
    risk_free_rate: float = 0.0,
    periods_per_year: float | None = None,
    symbol: str | None = None,
) -> BacktestMetrics:
    """Compute equity-curve and trade-level metrics from replay rows.

    ``rows`` should come from a replay run with ``track_portfolio=True`` (so the equity
    mark is populated). When the equity curve is absent, equity-derived metrics are zero
    but trade statistics are still computed from any fills present.
    """
    metrics = BacktestMetrics()
    ppy = periods_per_year if periods_per_year is not None else annualization_factor(
        bar_interval_seconds
    )
    metrics.periods_per_year = ppy

    _, equity = extract_equity_curve(rows)
    metrics.n_bars = len(equity)
    if equity:
        metrics.initial_equity = equity[0]
        metrics.final_equity = equity[-1]
        metrics.cumulative_pnl = equity[-1] - equity[0]
        if equity[0] != 0:
            metrics.total_return = (equity[-1] - equity[0]) / equity[0]
        rets = periodic_returns(equity)
        metrics.annualized_volatility = _stdev(rets) * math.sqrt(ppy) if rets else 0.0
        metrics.sharpe_ratio = sharpe_ratio(
            rets, periods_per_year=ppy, risk_free_rate=risk_free_rate
        )
        metrics.sortino_ratio = sortino_ratio(
            rets, periods_per_year=ppy, risk_free_rate=risk_free_rate
        )
        metrics.max_drawdown = max_drawdown(equity)
        # CAGR from elapsed bars; Calmar = CAGR / maxDD. Annualizing a sub-day window
        # is meaningless (and numerically explosive), so require >= ~1 day of bars.
        if metrics.n_bars > 1 and ppy > 0 and equity[0] > 0 and equity[-1] > 0:
            years = (metrics.n_bars - 1) / ppy
            if years >= (1.0 / 365.0):
                try:
                    metrics.cagr = (equity[-1] / equity[0]) ** (1.0 / years) - 1.0
                except OverflowError:
                    metrics.cagr = 0.0
        if metrics.max_drawdown > 0:
            metrics.calmar_ratio = metrics.cagr / metrics.max_drawdown

    records = build_trade_ledger(rows, symbol=symbol)
    metrics.trade_stats = summarize_trades(records)
    return metrics

"""Prometheus metrics for latency, PnL, drawdown, order success."""

from __future__ import annotations

from prometheus_client import Counter, Gauge, Histogram

DECISION_LATENCY = Histogram(
    "nm_decision_latency_seconds",
    "End-to-end decision latency",
    buckets=(0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0),
)
PNL_USD = Gauge("nm_pnl_usd", "Mark-to-market PnL USD")
DRAWDOWN_PCT = Gauge("nm_drawdown_pct", "Current drawdown vs peak")
ORDER_SUCCESS = Counter("nm_orders_success_total", "Successful order submissions", ["adapter"])
ORDER_FAIL = Counter("nm_orders_fail_total", "Failed order submissions", ["adapter"])

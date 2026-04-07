from __future__ import annotations

from prometheus_client import Counter, Gauge, Histogram

MARKET_DATA_LATENCY_SECONDS = Histogram(
    "nautilus_market_data_latency_seconds",
    "Latency from market event timestamp to processing time",
    buckets=(0.001, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0),
)

DECISION_LATENCY_SECONDS = Histogram(
    "nautilus_decision_latency_seconds",
    "Latency for decision pipeline processing",
    buckets=(0.001, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0),
)

ORDER_SUCCESS_TOTAL = Counter(
    "nautilus_order_success_total",
    "Count of successfully submitted orders",
    labelnames=("adapter", "symbol"),
)

ORDER_FAILURE_TOTAL = Counter(
    "nautilus_order_failure_total",
    "Count of failed order submissions",
    labelnames=("adapter", "symbol", "reason"),
)

PORTFOLIO_PNL_USD = Gauge(
    "nautilus_portfolio_pnl_usd",
    "Current unrealized portfolio PnL in USD",
)

PORTFOLIO_DRAWDOWN_PCT = Gauge(
    "nautilus_portfolio_drawdown_pct",
    "Current portfolio drawdown percentage (0-1)",
)

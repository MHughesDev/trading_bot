"""Prometheus metrics for latency, PnL, drawdown, order success."""

from __future__ import annotations

from prometheus_client import Counter, Gauge, Histogram

DECISION_LATENCY = Histogram(
    "tb_decision_latency_seconds",
    "End-to-end decision latency",
    buckets=(0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0),
)
PNL_USD = Gauge("tb_pnl_usd", "Mark-to-market PnL USD")
DRAWDOWN_PCT = Gauge("tb_drawdown_pct", "Current drawdown vs peak")
ORDER_SUCCESS = Counter("tb_orders_success_total", "Successful order submissions", ["adapter"])
ORDER_FAIL = Counter("tb_orders_fail_total", "Failed order submissions", ["adapter"])
FEED_STALE_BLOCKS = Counter(
    "tb_feed_stale_blocks_total",
    "Trades blocked because feed last_message age exceeded stale threshold",
)
NORMALIZER_UNKNOWN = Counter(
    "tb_normalizer_unknown_messages_total",
    "WebSocket messages that did not normalize to a known contract",
)
QUESTDB_WRITE_FAIL = Counter(
    "tb_questdb_write_fail_total",
    "QuestDB writer failures (bars, traces, flush)",
    ["operation"],
)

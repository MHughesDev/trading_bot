"""Prometheus metrics for forecaster inference (FB-FR-PG6 / FB-PL-PG7)."""

from __future__ import annotations

from prometheus_client import Counter, Gauge, Histogram

FORECAST_BUILD_SECONDS = Histogram(
    "tb_forecast_build_seconds",
    "Time to build ForecastPacket",
    buckets=(0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0),
)
FORECAST_BUILD_FALLBACK = Counter(
    "tb_forecast_build_fallback_total",
    "Forecast build used fallback or abstained",
)
FORECAST_PACKET_ABSTAIN = Counter(
    "tb_forecast_packet_abstain_total",
    "ForecastPacket rejected by guard",
)
MODEL_VERSION_INFO = Gauge(
    "tb_model_version_info",
    "Model artifact version label (set to 1 for active path)",
    ["component", "version"],
)
POLICY_SHADOW_DELTA = Histogram(
    "tb_policy_shadow_target_exposure_delta",
    "Absolute delta between shadow policy and baseline target exposure",
    buckets=(0.0, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0),
)

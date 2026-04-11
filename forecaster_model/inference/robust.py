"""Inference guards: NaN/Inf, timing, Prometheus counters (FB-FR-PG6)."""

from __future__ import annotations

import math
import time
from typing import Callable

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.inference.guards import ForecastGuard, ForecastGuardConfig

try:
    from observability.forecaster_metrics import (
        FORECAST_BUILD_FALLBACK,
        FORECAST_BUILD_SECONDS,
        FORECAST_PACKET_ABSTAIN,
    )
except ImportError:  # pragma: no cover
    FORECAST_BUILD_SECONDS = None
    FORECAST_BUILD_FALLBACK = None
    FORECAST_PACKET_ABSTAIN = None


def _finite_packet(pkt: ForecastPacket) -> bool:
    def ok_seq(xs: list[float]) -> bool:
        return all(math.isfinite(x) for x in xs)

    return (
        ok_seq(pkt.q_low)
        and ok_seq(pkt.q_med)
        and ok_seq(pkt.q_high)
        and ok_seq(pkt.interval_width)
        and isinstance(pkt.confidence_score, (int, float))
        and math.isfinite(float(pkt.confidence_score))
    )


def safe_build_forecast_packet(
    build_fn: Callable[..., ForecastPacket],
    *args,
    timeout_seconds: float = 30.0,
    guard: ForecastGuard | None = None,
    **kwargs,
) -> tuple[ForecastPacket | None, list[str]]:
    """
    Run builder with wall-clock timeout (best-effort via signal not used — short builds only),
    NaN rejection, and optional ForecastGuard abstention.
    """
    reasons: list[str] = []
    t0 = time.perf_counter()
    try:
        pkt = build_fn(*args, **kwargs)
    except Exception as e:  # noqa: BLE001 — surface as fallback
        if FORECAST_BUILD_FALLBACK:
            FORECAST_BUILD_FALLBACK.inc()
        return None, [f"build_error:{type(e).__name__}"]
    elapsed = time.perf_counter() - t0
    if FORECAST_BUILD_SECONDS:
        FORECAST_BUILD_SECONDS.observe(elapsed)
    if elapsed > timeout_seconds:
        if FORECAST_BUILD_FALLBACK:
            FORECAST_BUILD_FALLBACK.inc()
        return None, ["timeout"]
    if not _finite_packet(pkt):
        if FORECAST_BUILD_FALLBACK:
            FORECAST_BUILD_FALLBACK.inc()
        return None, ["non_finite_output"]
    g = guard or ForecastGuard(ForecastGuardConfig())
    ok, r = g.check(pkt)
    if not ok:
        if FORECAST_PACKET_ABSTAIN:
            FORECAST_PACKET_ABSTAIN.inc()
        reasons.extend(r)
        return None, reasons
    return pkt, []


def numpy_finite(arr: np.ndarray) -> bool:
    return bool(np.isfinite(arr).all())

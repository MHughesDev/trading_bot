"""Drift / calibration monitoring helpers (FB-CAN-039).

APEX Monitoring spec §4.9 — realized-vs-theoretical edge proxies, calibration drift,
shadow/replay divergence trends. See docs/MONITORING_CANONICAL.MD.
"""

from __future__ import annotations

from collections import deque
from typing import Any

from prometheus_client import Counter, Gauge, Histogram

# FB-CAN-069: rolling samples for post-release live probation (percentiles over recent ticks)
_PROB_EDGE: deque[float] = deque(maxlen=50_000)
_PROB_DRIFT: deque[float] = deque(maxlen=50_000)
_PROB_FP: deque[float] = deque(maxlen=50_000)
_PROB_LAST_RELEASE: str | None = None


def reset_probation_sample_buffers(release_id: str) -> None:
    """Clear rolling buffers when monitoring a different active-live release."""
    global _PROB_LAST_RELEASE
    rid = str(release_id or "").strip()
    if not rid:
        return
    if _PROB_LAST_RELEASE == rid:
        return
    _PROB_EDGE.clear()
    _PROB_DRIFT.clear()
    _PROB_FP.clear()
    _PROB_LAST_RELEASE = rid


def percentile_95(values: list[float]) -> float | None:
    if not values:
        return None
    xs = sorted(values)
    i = int(round(0.95 * (len(xs) - 1)))
    i = max(0, min(i, len(xs) - 1))
    return float(xs[i])


def get_probation_rolling_samples(max_samples: int) -> tuple[list[float], list[float], list[float]]:
    """Last up to ``max_samples`` values per series (edge erosion only on trade_intent ticks)."""
    n = max(0, int(max_samples))
    return (
        list(_PROB_EDGE)[-n:] if _PROB_EDGE else [],
        list(_PROB_DRIFT)[-n:] if _PROB_DRIFT else [],
        list(_PROB_FP)[-n:] if _PROB_FP else [],
    )

# --- Edge erosion (spec §4.7 / §4.9 — realized vs theoretical proxy) ---
CANONICAL_THEORETICAL_EDGE_SCORE = Histogram(
    "tb_canonical_theoretical_edge_score",
    "Joint decision×trigger confidence proxy for expected edge (trade_intent ticks)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_EDGE_EROSION = Histogram(
    "tb_canonical_edge_erosion_score",
    "max(0, theoretical_edge_proxy - execution_confidence) on trade_intent ticks",
    ["symbol"],
    buckets=(0.0, 0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0),
)
CANONICAL_EXECUTION_CONFIDENCE = Histogram(
    "tb_canonical_trade_intent_execution_confidence",
    "Execution confidence on emitted trade_intent (decision record)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)

# --- Feature / output drift proxies ---
CANONICAL_FEATURE_DRIFT_PENALTY = Histogram(
    "tb_canonical_feature_drift_penalty",
    "Decision-quality penalty from execution feedback memory (0-1, higher=worse)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_FALSE_POSITIVE_MEMORY = Histogram(
    "tb_canonical_trigger_false_positive_memory",
    "RiskState trigger false-positive memory activation rate (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)

# --- Calibration (conformal / confidence alignment) ---
CANONICAL_CONFIDENCE_CALIBRATION_GAP = Histogram(
    "tb_canonical_confidence_calibration_gap",
    "abs(route.confidence - forecaster confidence scalar) when both available",
    ["symbol"],
    buckets=(0.0, 0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0),
)
CANONICAL_CONFORMAL_INTERVAL_REL_WIDTH = Histogram(
    "tb_canonical_conformal_interval_relative_width",
    "Mean relative quantile band width (q_high-q_low)/max(|q_med|,eps) when conformal active",
    ["symbol"],
    buckets=(0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0),
)
CANONICAL_CONFORMAL_APPLIED = Counter(
    "tb_canonical_conformal_calibration_applied_total",
    "Ticks where conformal calibration widened quantiles (from forecast diagnostics)",
    ["symbol"],
)

# --- Shadow / replay divergence trend (from persisted governance store) ---
SHADOW_DIVERGENCE_RATE_GAUGE = Gauge(
    "tb_canonical_shadow_divergence_rate",
    "Last persisted shadow comparison divergence rate (see rate_kind label)",
    ["rate_kind"],
)
SHADOW_COMPARISON_WITHIN_THRESHOLDS = Gauge(
    "tb_canonical_shadow_comparison_within_thresholds",
    "1 if last shadow report within policy thresholds, else 0",
    [],
)
SHADOW_ROLLBACK_RECOMMENDED = Gauge(
    "tb_canonical_shadow_rollback_recommended",
    "1 if last shadow report recommends rollback, else 0",
    [],
)
SHADOW_COMPARISON_BARS = Gauge(
    "tb_canonical_shadow_comparison_bars_compared",
    "Bars compared in last persisted shadow report",
    [],
)


def _scalar_confidence(pkt: Any) -> float | None:
    cs = getattr(pkt, "confidence_score", None)
    if cs is None:
        return None
    if isinstance(cs, (list, tuple)):
        if not cs:
            return None
        return float(sum(float(x) for x in cs) / len(cs))
    try:
        return float(cs)
    except (TypeError, ValueError):
        return None


def record_calibration_and_drift_from_tick(
    *,
    symbol: str,
    risk: Any,
    forecast_packet: Any | None,
    feature_row: dict[str, float] | None,
    record_probation_samples: bool = True,
) -> None:
    """Record FB-CAN-039 metrics for one decision cycle (called from record_canonical_post_tick)."""
    sym = symbol or "unknown"
    feats = feature_row or {}

    fp_mem = float(getattr(risk, "trigger_false_positive_memory", 0.0) or 0.0)
    CANONICAL_FALSE_POSITIVE_MEMORY.labels(symbol=sym).observe(fp_mem)
    if record_probation_samples:
        _PROB_FP.append(fp_mem)

    dq = feats.get("canonical_exec_quality_penalty")
    if dq is not None:
        try:
            dq_f = float(dq)
            CANONICAL_FEATURE_DRIFT_PENALTY.labels(symbol=sym).observe(dq_f)
            if record_probation_samples:
                _PROB_DRIFT.append(dq_f)
        except (TypeError, ValueError):
            pass

    if forecast_packet is not None:
        fd = forecast_packet.forecast_diagnostics or {}
        if bool(fd.get("conformal_applied")):
            CANONICAL_CONFORMAL_APPLIED.labels(symbol=sym).inc()
            hl = list(getattr(forecast_packet, "horizons", []) or [])
            qlo = list(getattr(forecast_packet, "q_low", []) or [])
            qmd = list(getattr(forecast_packet, "q_med", []) or [])
            qhi = list(getattr(forecast_packet, "q_high", []) or [])
            n = min(len(hl), len(qlo), len(qmd), len(qhi))
            if n > 0:
                rels: list[float] = []
                for i in range(n):
                    lo, md, hi = float(qlo[i]), float(qmd[i]), float(qhi[i])
                    denom = max(abs(md), 1e-9)
                    rels.append((hi - lo) / denom)
                if rels:
                    CANONICAL_CONFORMAL_INTERVAL_REL_WIDTH.labels(symbol=sym).observe(
                        float(sum(rels) / len(rels))
                    )

    rec = getattr(risk, "last_decision_record", None)
    if not isinstance(rec, dict):
        return

    fc_sum = rec.get("forecast_summary") if isinstance(rec.get("forecast_summary"), dict) else {}
    fc_conf = _scalar_confidence(forecast_packet) if forecast_packet is not None else None
    rcv = fc_sum.get("route_confidence")
    if rcv is not None and fc_conf is not None:
        try:
            CANONICAL_CONFIDENCE_CALIBRATION_GAP.labels(symbol=sym).observe(
                abs(float(rcv) - float(fc_conf))
            )
        except (TypeError, ValueError):
            pass

    outcome = str(rec.get("outcome") or "")
    if outcome != "trade_intent":
        return
    ti = rec.get("trade_intent")
    if not isinstance(ti, dict):
        return
    try:
        dc = float(ti.get("decision_confidence", 0.0))
        tc = float(ti.get("trigger_confidence", 0.0))
        ec = float(ti.get("execution_confidence", 0.0))
    except (TypeError, ValueError):
        return
    theo = max(0.0, min(1.0, dc * tc))
    erosion = max(0.0, theo - ec)
    CANONICAL_THEORETICAL_EDGE_SCORE.labels(symbol=sym).observe(theo)
    CANONICAL_EDGE_EROSION.labels(symbol=sym).observe(erosion)
    CANONICAL_EXECUTION_CONFIDENCE.labels(symbol=sym).observe(ec)
    if record_probation_samples:
        _PROB_EDGE.append(erosion)


def refresh_shadow_divergence_gauges_from_store(store: dict[str, Any] | None) -> None:
    """Update shadow/replay trend gauges from governance shadow comparison store JSON."""
    if not isinstance(store, dict):
        return
    last = store.get("last_report")
    if not isinstance(last, dict):
        return
    rates = last.get("rates")
    if isinstance(rates, dict):
        for k, v in rates.items():
            try:
                SHADOW_DIVERGENCE_RATE_GAUGE.labels(rate_kind=str(k)).set(float(v))
            except (TypeError, ValueError):
                continue
    try:
        n = int(last.get("bars_compared", 0))
        SHADOW_COMPARISON_BARS.set(float(n))
    except (TypeError, ValueError):
        pass
    wt = last.get("within_thresholds")
    SHADOW_COMPARISON_WITHIN_THRESHOLDS.set(1.0 if wt is True else 0.0)
    rb = last.get("rollback_recommended")
    SHADOW_ROLLBACK_RECOMMENDED.set(1.0 if rb is True else 0.0)

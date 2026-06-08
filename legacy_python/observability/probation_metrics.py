"""Post-release live probation gauges (FB-CAN-069)."""

from __future__ import annotations

from prometheus_client import Counter, Gauge

PROBATION_MONITORING_ACTIVE = Gauge(
    "tb_governance_probation_monitoring_active",
    "1 when an active_live live release is inside the probation calendar window",
    [],
)
PROBATION_ABORT_RECOMMENDED = Gauge(
    "tb_governance_probation_abort_recommended",
    "1 when rolling risk-quality proxies breach policy during active probation phase",
    [],
)
PROBATION_PHASE_TICKS = Counter(
    "tb_governance_probation_phase_total",
    "Decision ticks attributed to probation phase",
    ["phase"],
)


def record_probation_gauges(*, active: bool, abort_recommended: bool, release_id: str) -> None:
    """Update gauges (``release_id`` reserved for future labeled metrics / logging)."""
    _ = release_id
    PROBATION_MONITORING_ACTIVE.set(1.0 if active else 0.0)
    PROBATION_ABORT_RECOMMENDED.set(1.0 if abort_recommended else 0.0)


def record_probation_tick(*, phase: str) -> None:
    PROBATION_PHASE_TICKS.labels(phase=str(phase or "unknown")).inc()


# Prime label sets so the metric registers with Prometheus before first live tick.
for _ph in ("inactive", "active", "cooldown", "unknown"):
    PROBATION_PHASE_TICKS.labels(phase=_ph).inc(0)

__all__ = [
    "PROBATION_ABORT_RECOMMENDED",
    "PROBATION_MONITORING_ACTIVE",
    "PROBATION_PHASE_TICKS",
    "record_probation_gauges",
    "record_probation_tick",
]

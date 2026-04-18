"""Post-release live probation evaluation (FB-CAN-069).

Uses active **live** release ledger candidates, rolling decision-tick samples from
``observability.drift_calibration_metrics``, and ``apex_canonical.domains.post_release_probation``
thresholds to set Prometheus gauges and recommend abort/rollback during the early window.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from app.config.post_release_probation import probation_policy_from_settings
from app.contracts.release_objects import ReleaseCandidate, read_release_ledger
from observability.drift_calibration_metrics import (
    get_probation_rolling_samples,
    percentile_95,
    reset_probation_sample_buffers,
)
from observability.probation_metrics import record_probation_gauges, record_probation_tick

_LAST_PREPARED_RELEASE: str | None = None


def prepare_probation_sample_buffers_before_metrics(
    *,
    settings: Any,
    ledger_path: str | None = None,
) -> None:
    """Call **before** ``record_canonical_post_tick`` so a release switch clears prior samples only."""
    global _LAST_PREPARED_RELEASE
    try:
        policy = probation_policy_from_settings(settings)
    except Exception:
        return
    if not policy.enabled:
        return
    ledger = read_release_ledger(ledger_path)
    cand = _active_live_candidate(ledger)
    if cand is None:
        _LAST_PREPARED_RELEASE = None
        return
    rid = str(cand.release_id)
    if _LAST_PREPARED_RELEASE is None:
        _LAST_PREPARED_RELEASE = rid
        return
    if _LAST_PREPARED_RELEASE != rid:
        reset_probation_sample_buffers(rid)
        _LAST_PREPARED_RELEASE = rid


def _parse_approved_at(raw: str | None) -> datetime | None:
    if not raw or not str(raw).strip():
        return None
    s = str(raw).strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(s)
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=UTC)
    return dt


def _active_live_candidate(ledger: Any) -> ReleaseCandidate | None:
    if ledger is None:
        return None
    best: ReleaseCandidate | None = None
    best_ts: datetime | None = None
    for c in getattr(ledger, "candidates", None) or []:
        if c.environment != "live":
            continue
        if c.current_stage != "active_live":
            continue
        ap = _parse_approved_at(c.approved_at)
        if ap is None:
            continue
        if best is None or ap > (best_ts or ap):
            best = c
            best_ts = ap
    return best


def evaluate_post_release_probation_tick(
    *,
    settings: Any,
    risk_engine: Any,
    now: datetime | None = None,
    ledger_path: str | None = None,
) -> dict[str, Any]:
    """
    Call once per decision tick (live path) after ``record_canonical_post_tick`` has run
    so rolling samples include this tick.
    """
    _ = risk_engine
    policy = probation_policy_from_settings(settings)
    if not policy.enabled:
        record_probation_gauges(active=False, abort_recommended=False, release_id="")
        return {"enabled": False, "note": "probation_disabled"}

    tnow = now or datetime.now(UTC)
    if tnow.tzinfo is None:
        tnow = tnow.replace(tzinfo=UTC)

    ledger = read_release_ledger(ledger_path)
    cand = _active_live_candidate(ledger)
    if cand is None:
        record_probation_gauges(active=False, abort_recommended=False, release_id="")
        return {"enabled": True, "active": False, "note": "no_active_live_candidate"}

    meta_lv = None
    try:
        meta_lv = settings.canonical.metadata.logic_version
    except Exception:
        pass
    if meta_lv and cand.logic_version and str(cand.logic_version).strip() != str(meta_lv).strip():
        record_probation_gauges(active=False, abort_recommended=False, release_id="")
        return {
            "enabled": True,
            "active": False,
            "note": "logic_version_mismatch",
            "candidate_logic_version": cand.logic_version,
            "metadata_logic_version": meta_lv,
        }

    approved = _parse_approved_at(cand.approved_at)
    if approved is None:
        record_probation_gauges(active=False, abort_recommended=False, release_id="")
        return {"enabled": True, "active": False, "note": "approved_at_unparseable"}

    rid = str(cand.release_id)

    elapsed_sec = (tnow - approved).total_seconds()
    w = policy.windows
    total_sec = max(1.0, float(w.total_window_hours) * 3600.0)
    active_sec = max(1.0, float(w.active_hours) * 3600.0)
    cool_sec = max(0.0, float(w.cooldown_hours) * 3600.0)

    if elapsed_sec < 0 or elapsed_sec > total_sec:
        record_probation_gauges(active=False, abort_recommended=False, release_id=rid)
        return {
            "enabled": True,
            "active": False,
            "release_id": rid,
            "note": "outside_probation_calendar_window",
            "elapsed_seconds": elapsed_sec,
        }

    phase = "inactive"
    if elapsed_sec <= active_sec:
        phase = "active"
    elif cool_sec > 0 and elapsed_sec <= active_sec + cool_sec:
        phase = "cooldown"
    else:
        phase = "inactive"

    record_probation_tick(phase=phase)

    edge_s, drift_s, fp_s = get_probation_rolling_samples(policy.sample_window_ticks)
    n_edge = len(edge_s)

    abort = False
    reasons: list[str] = []
    thr = policy.thresholds
    if phase == "active" and n_edge >= 32:
        e95 = percentile_95(edge_s)
        d95 = percentile_95(drift_s) if drift_s else None
        fp95 = percentile_95(fp_s) if fp_s else None

        if e95 is not None and e95 > thr.edge_erosion_p95_max:
            abort = True
            reasons.append(f"edge_erosion_p95 {e95:.4f} > {thr.edge_erosion_p95_max}")
        if d95 is not None and d95 > thr.feature_drift_penalty_p95_max:
            abort = True
            reasons.append(f"feature_drift_penalty_p95 {d95:.4f} > {thr.feature_drift_penalty_p95_max}")
        if fp95 is not None and fp95 > thr.trigger_false_positive_memory_p95_max:
            abort = True
            reasons.append(
                f"trigger_false_positive_memory_p95 {fp95:.4f} > {thr.trigger_false_positive_memory_p95_max}"
            )

    in_monitoring = phase in ("active", "cooldown")
    record_probation_gauges(
        active=in_monitoring,
        abort_recommended=abort,
        release_id=rid,
    )

    return {
        "enabled": True,
        "active": in_monitoring,
        "phase": phase,
        "release_id": rid,
        "approved_at": cand.approved_at,
        "elapsed_seconds": elapsed_sec,
        "abort_recommended": abort,
        "reasons": reasons,
        "samples_trade_intent_edge": n_edge,
        "policy": policy.model_dump(mode="json"),
    }


__all__ = [
    "evaluate_post_release_probation_tick",
    "prepare_probation_sample_buffers_before_metrics",
]

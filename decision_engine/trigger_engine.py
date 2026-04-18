"""APEX three-stage trigger engine (FB-CAN-005, FB-CAN-043).

Setup → Pre-Trigger → Confirmed Trigger with deterministic scores, missed-move suppression,
and reason codes. See APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

from app.contracts.reason_codes import (
    TRG_DEGRADATION_BLOCK,
    TRG_EXECUTION_TOO_DEGRADED,
    TRG_INSUFFICIENT_REMAINING_EDGE,
    TRG_LOW_SETUP_SCORE,
    TRG_MOVE_ALREADY_EXTENDED,
    TRG_NOVELTY_BLOCK,
    TRG_POOR_EXECUTION_CONTEXT,
    TRG_PRESSURE_NOT_BUILDING,
    TRG_STALE_PRETRIGGER_INPUTS,
    TRG_TRIGGER_STRENGTH_LOW,
)
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.canonical_structure import CanonicalStructureOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.structure_adapter import structure_from_forecast_packet
from app.contracts.trigger import TriggerOutput


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def _safe_float(row: dict[str, float], key: str, default: float = 0.0) -> float:
    v = row.get(key)
    if v is None:
        return default
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


def _structural_confidence(pkt: ForecastPacket) -> float:
    cs = pkt.confidence_score
    if isinstance(cs, list):
        return _clip01(sum(cs) / max(len(cs), 1))
    return _clip01(float(cs))


def asymmetry_score(pkt: ForecastPacket) -> float:
    """Directional asymmetry from short-horizon quantiles [0,1]."""
    if not pkt.q_low or not pkt.q_high or not pkt.q_med:
        return 0.0
    lo, hi, med = float(pkt.q_low[0]), float(pkt.q_high[0]), float(pkt.q_med[0])
    width = max(hi - lo, 1e-12)
    bias = abs(((med - lo) / width) - 0.5) * 2.0
    return _clip01(bias)


def state_alignment_score(apex: CanonicalStateOutput) -> float:
    rp = apex.regime_probabilities
    if len(rp) >= 5:
        return _clip01(max(rp[0], rp[1]))
    return 0.5


def evaluate_trigger(
    pkt: ForecastPacket,
    feature_row: dict[str, float],
    *,
    spread_bps: float,
    apex: CanonicalStateOutput,
    structure: CanonicalStructureOutput | None = None,
    decision_timestamp: datetime | None = None,
) -> TriggerOutput:
    """Evaluate three-stage trigger from packet, microstructure proxies, and canonical state."""
    reasons: list[str] = []
    stage_fail: dict[str, list[str]] = {"setup": [], "pretrigger": [], "confirm": []}
    st = structure if structure is not None else structure_from_forecast_packet(pkt)

    A = float(st.asymmetry_score)
    S = state_alignment_score(apex)
    C_struct = _clip01(0.55 * float(st.model_agreement_score) + 0.45 * _structural_confidence(pkt))
    C = _clip01(0.5 * apex.regime_confidence + 0.5 * C_struct)
    H = apex.heat_score
    N = apex.novelty
    Rfx = apex.reflexivity_score

    wA, wS, wC, wH, wN, wRfx = 0.35, 0.25, 0.3, 0.35, 0.35, 0.25
    setup_raw = wA * A + wS * S + wC * C - wH * H - wN * N - wRfx * Rfx
    setup_score = _clip01(setup_raw)

    spread_stress = _clip01(spread_bps / 80.0)
    exec_conf = _clip01(1.0 - spread_stress * 0.8)
    dq = _safe_float(feature_row, "canonical_exec_quality_penalty", 0.0)
    exec_conf = _clip01(exec_conf * (1.0 - 0.55 * _clip01(dq)))
    # Permissive defaults for stub/RNG paths; tighten when wiring canonical config.
    setup_threshold = 0.22
    setup_exec_floor = 0.12

    novelty_hard = N >= 0.98
    if novelty_hard:
        reasons.append(TRG_NOVELTY_BLOCK)
        stage_fail["setup"].append(TRG_NOVELTY_BLOCK)

    if apex.degradation == DegradationLevel.NO_TRADE:
        reasons.append(TRG_DEGRADATION_BLOCK)
        stage_fail["setup"].append(TRG_DEGRADATION_BLOCK)

    setup_valid = (
        setup_score >= setup_threshold
        and apex.degradation != DegradationLevel.NO_TRADE
        and not novelty_hard
        and exec_conf >= setup_exec_floor
    )
    if not setup_valid and TRG_NOVELTY_BLOCK not in reasons and TRG_DEGRADATION_BLOCK not in reasons:
        if setup_score < setup_threshold:
            reasons.append(TRG_LOW_SETUP_SCORE)
            stage_fail["setup"].append(TRG_LOW_SETUP_SCORE)
        if exec_conf < setup_exec_floor:
            reasons.append(TRG_POOR_EXECUTION_CONTEXT)
            stage_fail["setup"].append(TRG_POOR_EXECUTION_CONTEXT)

    rsi = _safe_float(feature_row, "rsi_14", 50.0)
    ret1 = _safe_float(feature_row, "return_1", 0.0)
    vol = _safe_float(feature_row, "volume", 0.0)
    vol_norm = _clip01(vol / (abs(_safe_float(feature_row, "close", 1.0)) * 1e-6 + 1e6))

    imb_shift = _clip01(abs(rsi - 50.0) / 50.0)
    vol_score = vol_norm
    T_score = _clip01(1.0 - spread_stress)
    F = _clip01(1.0 - float(pkt.ood_score))
    F = _clip01(0.5 * F + 0.5 * (1.0 - float(st.fragility_score)))

    wI, wV, wT, wF = 0.3, 0.25, 0.25, 0.2
    pre_raw = wI * imb_shift + wV * vol_score + wT * T_score + wF * F
    pretrigger_score = _clip01(pre_raw)
    pretrigger_threshold = 0.18
    freshness_floor = 0.08
    pretrigger_valid = (
        setup_valid
        and pretrigger_score >= pretrigger_threshold
        and F >= freshness_floor
    )
    if setup_valid and not pretrigger_valid:
        if pretrigger_score < pretrigger_threshold:
            reasons.append(TRG_PRESSURE_NOT_BUILDING)
            stage_fail["pretrigger"].append(TRG_PRESSURE_NOT_BUILDING)
        if F < freshness_floor:
            reasons.append(TRG_STALE_PRETRIGGER_INPUTS)
            stage_fail["pretrigger"].append(TRG_STALE_PRETRIGGER_INPUTS)

    B = _clip01(abs(ret1) * 25.0)
    U = vol_score
    K = _clip01(float(pkt.interval_width[0]) if pkt.interval_width else 0.0)
    c1, c2, c3 = 0.35, 0.35, 0.3
    composite = c1 * B + c2 * U + c3 * K
    trigger_strength = _clip01(max(B, U, K, composite))

    trigger_threshold = 0.2
    trigger_exec_floor = 0.1
    entry_extension_limit = 0.85
    min_remaining_edge = 0.03

    # Spec §8 — missed move evaluated after strength, before final trigger_valid (pseudocode §12).
    width0 = float(pkt.interval_width[0]) if pkt.interval_width else 0.0
    E = _clip01(width0 * 2.0)
    R = A
    X = spread_stress * 0.5 + (1.0 - exec_conf) * 0.5
    missed_move_flag = E > entry_extension_limit or (R - X) < min_remaining_edge
    if missed_move_flag:
        if E > entry_extension_limit:
            reasons.append(TRG_MOVE_ALREADY_EXTENDED)
            stage_fail["confirm"].append(TRG_MOVE_ALREADY_EXTENDED)
        else:
            reasons.append(TRG_INSUFFICIENT_REMAINING_EDGE)
            stage_fail["confirm"].append(TRG_INSUFFICIENT_REMAINING_EDGE)

    strength_ok = (
        setup_valid
        and pretrigger_valid
        and trigger_strength >= trigger_threshold
        and exec_conf >= trigger_exec_floor
    )
    if setup_valid and pretrigger_valid and not strength_ok:
        if exec_conf < trigger_exec_floor:
            reasons.append(TRG_EXECUTION_TOO_DEGRADED)
            stage_fail["confirm"].append(TRG_EXECUTION_TOO_DEGRADED)
        elif trigger_strength < trigger_threshold:
            reasons.append(TRG_TRIGGER_STRENGTH_LOW)
            stage_fail["confirm"].append(TRG_TRIGGER_STRENGTH_LOW)

    trigger_valid = strength_ok and not missed_move_flag

    trig_type = "none"
    if trigger_valid:
        candidates = [
            ("imbalance_spike", B),
            ("volume_burst", U),
            ("structure_break", K),
            ("composite_confirmed", composite),
        ]
        best_name = "composite_confirmed"
        best_val = -1.0
        for name, v in candidates:
            if v >= best_val:
                best_name, best_val = name, v
        trig_type = best_name

    Cf = C_struct
    Cm = _clip01(1.0 - spread_stress)
    Ce = exec_conf
    Cd = F
    trigger_confidence = _clip01((Cf + Cm + Ce + Cd) / 4.0)

    ref_ts = decision_timestamp if decision_timestamp is not None else pkt.timestamp
    if ref_ts.tzinfo is None:
        ref_ts = ref_ts.replace(tzinfo=UTC)
    else:
        ref_ts = ref_ts.astimezone(UTC)
    # Deterministic sub-tick ordering for replay (not wall-clock profiling)
    t_setup = ref_ts
    t_pre = ref_ts + timedelta(milliseconds=1)
    t_conf = ref_ts + timedelta(milliseconds=2)
    lat_ms = 2.0

    return TriggerOutput(
        setup_valid=setup_valid,
        setup_score=setup_score,
        pretrigger_valid=pretrigger_valid,
        pretrigger_score=pretrigger_score,
        trigger_valid=trigger_valid,
        trigger_type=trig_type,
        trigger_strength=trigger_strength,
        trigger_confidence=trigger_confidence,
        missed_move_flag=missed_move_flag,
        trigger_reason_codes=reasons,
        stage_timestamp_setup=t_setup.replace(microsecond=0).isoformat(),
        stage_timestamp_pretrigger=t_pre.replace(microsecond=0).isoformat(),
        stage_timestamp_confirm=t_conf.replace(microsecond=0).isoformat(),
        setup_to_confirm_latency_ms=lat_ms,
        stage_failure_codes=stage_fail,
    )

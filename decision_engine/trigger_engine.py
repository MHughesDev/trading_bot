"""APEX three-stage trigger engine (FB-CAN-005).

Setup → Pre-Trigger → Confirmed Trigger with deterministic scores, missed-move suppression,
and reason codes. See APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md.
"""

from __future__ import annotations

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
) -> TriggerOutput:
    """Evaluate three-stage trigger from packet, microstructure proxies, and canonical state."""
    reasons: list[str] = []
    st = structure if structure is not None else structure_from_forecast_packet(pkt)

    A = float(st.asymmetry_score)
    S = state_alignment_score(apex)
    C_struct = _clip01(0.55 * float(st.model_agreement_score) + 0.45 * _structural_confidence(pkt))
    C = _clip01(0.5 * apex.regime_confidence + 0.5 * C_struct)
    H = apex.heat_score
    N = apex.novelty

    wA, wS, wC, wH, wN = 0.35, 0.25, 0.3, 0.35, 0.35
    setup_raw = wA * A + wS * S + wC * C - wH * H - wN * N
    setup_score = _clip01(setup_raw)

    spread_stress = _clip01(spread_bps / 80.0)
    exec_conf = _clip01(1.0 - spread_stress * 0.8)
    # Permissive defaults for stub/RNG paths; tighten when wiring canonical config.
    setup_threshold = 0.22
    setup_exec_floor = 0.12

    novelty_hard = N >= 0.98
    if novelty_hard:
        reasons.append("novelty_block")

    if apex.degradation == DegradationLevel.NO_TRADE:
        reasons.append("degradation_block")

    setup_valid = (
        setup_score >= setup_threshold
        and apex.degradation != DegradationLevel.NO_TRADE
        and not novelty_hard
        and exec_conf >= setup_exec_floor
    )
    if not setup_valid and "novelty_block" not in reasons and "degradation_block" not in reasons:
        if setup_score < setup_threshold:
            reasons.append("low_setup_score")
        if exec_conf < setup_exec_floor:
            reasons.append("poor_execution_context")

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
            reasons.append("pressure_not_building")
        if F < freshness_floor:
            reasons.append("stale_pretrigger_inputs")

    B = _clip01(abs(ret1) * 25.0)
    U = vol_score
    K = _clip01(float(pkt.interval_width[0]) if pkt.interval_width else 0.0)
    c1, c2, c3 = 0.35, 0.35, 0.3
    composite = c1 * B + c2 * U + c3 * K
    trigger_strength = _clip01(max(B, U, K, composite))

    trigger_threshold = 0.2
    trigger_exec_floor = 0.1
    trigger_valid = (
        setup_valid
        and pretrigger_valid
        and trigger_strength >= trigger_threshold
        and exec_conf >= trigger_exec_floor
    )

    if not trigger_valid:
        trig_type = "none"
    else:
        # Deterministic tie-break: imbalance → volume → structure → composite (last wins ties on max)
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

    width0 = float(pkt.interval_width[0]) if pkt.interval_width else 0.0
    E = _clip01(width0 * 2.0)
    R = A
    X = spread_stress * 0.5 + (1.0 - exec_conf) * 0.5
    missed_move_flag = E > 0.85 or (R - X) < 0.03
    if missed_move_flag:
        reasons.append("move_already_extended" if E > 0.85 else "insufficient_remaining_edge")
        trigger_valid = False

    if not trigger_valid and setup_valid and pretrigger_valid and not missed_move_flag:
        if trigger_strength < trigger_threshold:
            reasons.append("trigger_strength_low")

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
    )

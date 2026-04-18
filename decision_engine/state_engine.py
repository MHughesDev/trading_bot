"""APEX canonical state / regime engine (FB-CAN-004).

Builds :class:`app.contracts.canonical_state.CanonicalStateOutput` from forecast packet + features.
Deterministic, replay-friendly; structural inputs default to neutral when absent from feature_row.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from app.config.settings import AppSettings
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.canonical_structure import CanonicalStructureOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.hard_override import HardOverrideKind
from app.contracts.risk import RiskState, SystemMode
from app.contracts.trigger import TriggerOutput
from decision_engine.trigger_engine import asymmetry_score
from risk_engine.canonical_sizing import classify_liquidation_mode, exec_confidence_from_spread


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


def _normalize5(raw: list[float]) -> list[float]:
    s = sum(max(0.0, x) for x in raw) or 1.0
    return [max(0.0, x) / s for x in raw]


def _state_safety_domain(settings: AppSettings | None) -> dict[str, Any]:
    if settings is None:
        return {}
    try:
        d = settings.canonical.domains.state_safety_degradation
        return dict(d) if d is not None else {}
    except Exception:
        return {}


def _normalize_named_weights(
    raw: dict[str, Any] | None,
    keys: tuple[str, ...],
    defaults: dict[str, float],
) -> dict[str, float]:
    src = raw if isinstance(raw, dict) else {}
    out: dict[str, float] = {}
    for k in keys:
        out[k] = float(src.get(k, defaults[k]))
    s = sum(max(0.0, v) for v in out.values()) or 1.0
    return {k: max(0.0, out[k]) / s for k in keys}


def _regime_confidence_separation(probs: list[float]) -> float:
    """APEX State spec §6 — max(R) - second_max(R) on the 5-class vector."""
    if len(probs) < 2:
        return _clip01(probs[0] if probs else 0.0)
    sp = sorted(probs, reverse=True)
    return _clip01(sp[0] - sp[1])


def build_canonical_state(
    pkt: ForecastPacket,
    feature_row: dict[str, float],
    *,
    spread_bps: float,
    settings: AppSettings | None = None,
    structure: CanonicalStructureOutput | None = None,
) -> CanonicalStateOutput:
    """Derive APEX canonical state from forecaster packet and microstructure features.

    **5-class order (spec §4–5):** trend, range, stress, dislocated, transition.
    **Transition scalar (spec §7.3):** ``Rc`` inverse regime confidence (HMM 4-way), ``Vt`` vol transition,
    ``Md`` microstructure disagreement, ``Sd`` structural disagreement — clipped to ``[0, 1]``.
    """
    rv = list(pkt.regime_vector)
    if len(rv) < 4:
        rv = (rv + [0.25] * 4)[:4]
    s4 = sum(rv) or 1.0
    p = [max(0.0, x) / s4 for x in rv[:4]]
    p_bull, p_bear, p_vol, p_side = p[0], p[1], p[2], p[3]

    close = max(_safe_float(feature_row, "close", 1.0), 1e-12)
    atr = _safe_float(feature_row, "atr_14", 0.0)
    rel_vol = _clip01(atr / close * 50.0)

    spread_stress = _clip01(spread_bps / 80.0)

    # HMM 4-way separation (feeds Rc for transition risk before 5-class merge)
    sorted_hmm = sorted(p, reverse=True)
    hmm_conf = (sorted_hmm[0] - sorted_hmm[1]) if len(sorted_hmm) > 1 else sorted_hmm[0]
    rc = _clip01(1.0 - _clip01(hmm_conf))

    vt = rel_vol
    md = _clip01(abs(p_bull - p_bear))
    sd = _clip01(1.0 - max(p_bull + p_bear, p_side + p_vol))
    # Spec §7.3 — coefficients sum to 1.0 for interpretability
    t_raw = 0.35 * rc + 0.25 * vt + 0.20 * md + 0.20 * sd
    transition_probability = _clip01(t_raw)

    # Unnormalized masses → normalize to R (spec §5)
    w_trend = max(1e-9, _clip01(p_bull + p_bear))
    w_range = max(1e-9, _clip01(p_side))
    w_stress = max(1e-9, _clip01(p_vol * 0.55 + rel_vol * 0.25))
    w_dislocated = max(1e-9, _clip01(p_vol * 0.20 + spread_stress * 0.35 + rel_vol * 0.20))
    w_transition = max(1e-9, transition_probability * 0.85 + (1.0 - max(p)) * 0.15)

    regime_probs = _normalize5([w_trend, w_range, w_stress, w_dislocated, w_transition])
    regime_confidence = _regime_confidence_separation(regime_probs)

    ood = _clip01(float(pkt.ood_score))
    hmm_amb = _clip01(1.0 - max(p))
    frag = _clip01(float(structure.fragility_score)) if structure is not None else 0.0

    dom = _state_safety_domain(settings)
    nov_def = {"ood": 0.38, "hmm_ambiguity": 0.22, "structure_fragility": 0.25, "transition": 0.15}
    nov_w = _normalize_named_weights(dom.get("novelty_weights"), tuple(nov_def.keys()), nov_def)
    novelty_components = {
        "ood": ood,
        "hmm_ambiguity": hmm_amb,
        "structure_fragility": frag,
        "transition": transition_probability,
    }
    novelty = _clip01(
        sum(nov_w[k] * novelty_components[k] for k in nov_w)
    )
    novelty_reason_codes: list[str] = []
    if ood >= 0.90:
        novelty_reason_codes.append("high_ood")
    if hmm_amb >= 0.75:
        novelty_reason_codes.append("hmm_ambiguous")
    if frag >= 0.72:
        novelty_reason_codes.append("structure_fragile")
    if transition_probability >= 0.65:
        novelty_reason_codes.append("elevated_transition_risk")

    rsi = _safe_float(feature_row, "rsi_14", 50.0)
    rsi_ext = _clip01(abs(rsi - 50.0) / 50.0)
    hf = rsi_ext * 0.2
    hl = rel_vol * 0.25
    ho = rel_vol * 0.15
    hx = spread_stress * 0.15
    hv = rel_vol * 0.2
    he = spread_stress * 0.15
    heat_components = {"Hf": hf, "Hl": hl, "Ho": ho, "Hx": hx, "Hv": hv, "He": he}
    heat_def = {"Hf": 0.2, "Hl": 0.25, "Ho": 0.15, "Hx": 0.15, "Hv": 0.15, "He": 0.1}
    hw = _normalize_named_weights(dom.get("heat_weights"), tuple(heat_def.keys()), heat_def)
    heat_raw = sum(hw[k] * heat_components[k] for k in hw)
    heat_score = _clip01(heat_raw)

    dir_press = _clip01(abs(float(structure.directional_bias))) if structure is not None else 0.0
    ref_def = {"rsi_ext": 0.3, "rel_vol": 0.25, "fragility": 0.25, "directional_pressure": 0.2}
    ref_w = _normalize_named_weights(dom.get("reflexivity_weights"), tuple(ref_def.keys()), ref_def)
    reflexivity_components = {
        "rsi_ext": rsi_ext,
        "rel_vol": rel_vol,
        "fragility": frag,
        "directional_pressure": dir_press,
    }
    reflexivity = _clip01(
        sum(ref_w[k] * reflexivity_components[k] for k in ref_w)
    )

    deg = DegradationLevel.NORMAL
    if spread_bps > 75 or heat_score > 0.92 or transition_probability > 0.88 or novelty >= 0.95:
        deg = DegradationLevel.NO_TRADE
    elif (
        heat_score > 0.68
        or transition_probability > 0.58
        or novelty > 0.82
        or reflexivity > 0.88
    ):
        deg = DegradationLevel.DEFENSIVE
    elif heat_score > 0.45 or transition_probability > 0.38 or reflexivity > 0.72:
        deg = DegradationLevel.REDUCED

    return CanonicalStateOutput(
        regime_probabilities=regime_probs,
        regime_confidence=regime_confidence,
        transition_probability=transition_probability,
        novelty=novelty,
        heat_score=heat_score,
        reflexivity_score=reflexivity,
        degradation=deg,
        heat_components=heat_components,
        novelty_components=novelty_components,
        reflexivity_components=reflexivity_components,
        novelty_reason_codes=novelty_reason_codes,
    )


def classify_hard_override(
    *,
    risk: RiskState,
    feature_row: dict[str, float] | None,
    spread_bps: float,
    settings: AppSettings,
    feed_last_message_at: datetime | None,
    data_timestamp: datetime | None,
    now_ref: datetime | None,
    product_tradable: bool = True,
) -> tuple[bool, HardOverrideKind]:
    """Deterministic hard-override classification (FB-CAN-033). First matching rule wins."""
    if risk.mode != SystemMode.RUNNING:
        return True, HardOverrideKind.SYSTEM_MODE

    if not product_tradable:
        return True, HardOverrideKind.PRODUCT_UNTRADABLE

    stale_sec = float(settings.risk_stale_data_seconds)
    max_spread = float(settings.risk_max_spread_bps)
    max_dd = float(settings.risk_max_drawdown_pct)
    ref = now_ref if now_ref is not None else datetime.now(UTC)
    if ref.tzinfo is None:
        ref = ref.replace(tzinfo=UTC)
    else:
        ref = ref.astimezone(UTC)

    if feed_last_message_at is not None:
        flm = (
            feed_last_message_at
            if feed_last_message_at.tzinfo
            else feed_last_message_at.replace(tzinfo=UTC)
        ).astimezone(UTC)
        if abs((ref - flm).total_seconds()) > stale_sec:
            return True, HardOverrideKind.FEED_STALE

    if data_timestamp is not None:
        dt = data_timestamp if data_timestamp.tzinfo else data_timestamp.replace(tzinfo=UTC)
        dt = dt.astimezone(UTC)
        if abs((ref - dt).total_seconds()) > stale_sec:
            return True, HardOverrideKind.DATA_TIMESTAMP_STALE

    if float(spread_bps) > max_spread:
        return True, HardOverrideKind.SPREAD_WIDE

    if float(risk.current_drawdown_pct) > max_dd:
        return True, HardOverrideKind.DRAWDOWN

    fr = feature_row or {}
    comp = fr.get("canonical_snapshot_complete")
    if comp is not None:
        try:
            if float(comp) < 0.4:
                return True, HardOverrideKind.NORMALIZATION_INCOMPLETE
        except (TypeError, ValueError):
            pass

    sc = fr.get("signal_confidence_aggregate")
    if sc is not None:
        try:
            if float(sc) < 0.25:
                return True, HardOverrideKind.SIGNAL_CONFIDENCE_LOW
        except (TypeError, ValueError):
            pass

    return False, HardOverrideKind.NONE


def _apply_degradation_transition_fields(
    risk: RiskState,
    new_level: DegradationLevel,
) -> dict[str, Any]:
    """Update transition count, last level string, and per-level occupancy ticks."""
    prev = risk.canonical_degradation
    new_s = new_level.value
    prev_s = prev.value if prev is not None else None

    trans_count = int(risk.degradation_transition_count)
    if prev_s is not None and prev_s != new_s:
        trans_count += 1

    occ = dict(risk.degradation_occupancy_ticks or {})
    if prev_s == new_s:
        occ[new_s] = int(occ.get(new_s, 0)) + 1
    else:
        for k in list(occ.keys()):
            occ[k] = 0
        occ[new_s] = 1

    return {
        "degradation_transition_count": trans_count,
        "last_degradation_level": new_s,
        "degradation_occupancy_ticks": occ,
    }


def apply_normalization_degradation(
    apex: CanonicalStateOutput,
    feature_row: dict[str, float] | None,
) -> CanonicalStateOutput:
    """FB-CAN-016: bump degradation when normalized feature confidence/completeness is low."""
    fr = feature_row or {}
    deg = apex.degradation
    sc = fr.get("signal_confidence_aggregate")
    if sc is not None and float(sc) < 0.25:
        if deg in (DegradationLevel.NORMAL, DegradationLevel.REDUCED):
            deg = DegradationLevel.REDUCED
    comp = fr.get("canonical_snapshot_complete")
    if comp is not None and float(comp) < 0.4:
        if deg == DegradationLevel.NORMAL:
            deg = DegradationLevel.REDUCED
    if deg == apex.degradation:
        return apex
    return apex.model_copy(update={"degradation": deg})


def degradation_size_multiplier(level: DegradationLevel) -> float:
    """Throttle notional by degradation (spec §6.4 style)."""
    return {
        DegradationLevel.NORMAL: 1.0,
        DegradationLevel.REDUCED: 0.75,
        DegradationLevel.DEFENSIVE: 0.5,
        DegradationLevel.NO_TRADE: 0.0,
    }[level]


def merge_canonical_into_risk(
    risk: Any,
    apex: CanonicalStateOutput | None,
    *,
    forecast_packet: ForecastPacket | None = None,
    trigger: TriggerOutput | None = None,
    spread_bps: float = 0.0,
    feature_row: dict[str, float] | None = None,
    hard_override_active: bool = False,
    hard_override_kind: HardOverrideKind = HardOverrideKind.NONE,
) -> Any:
    """Attach canonical degradation, size multiplier, and FB-CAN-007 sizing inputs."""
    if not isinstance(risk, RiskState):
        return risk
    fr = feature_row or {}
    norm_fields: dict[str, Any] = {}
    for k in (
        "feature_freshness",
        "feature_reliability",
        "signal_confidence_aggregate",
        "canonical_snapshot_complete",
    ):
        if k in fr:
            try:
                norm_fields[k] = float(fr[k])
            except (TypeError, ValueError):
                pass
    if norm_fields:
        risk = risk.model_copy(update=norm_fields)

    if apex is None:
        return risk.model_copy(
            update={
                "hard_override_active": bool(hard_override_active),
                "hard_override_kind": hard_override_kind,
            }
        )

    upd: dict[str, Any] = {
        "canonical_degradation": apex.degradation,
        "canonical_size_multiplier": degradation_size_multiplier(apex.degradation),
        "hard_override_active": bool(hard_override_active),
        "hard_override_kind": hard_override_kind,
        **_apply_degradation_transition_fields(risk, apex.degradation),
    }
    if forecast_packet is not None:
        close = max(_safe_float(feature_row or {}, "close", 1.0), 1e-12)
        atr = _safe_float(feature_row or {}, "atr_14", 0.0)
        atr_over = atr / close
        asym = asymmetry_score(forecast_packet)
        tc = float(trigger.trigger_confidence) if trigger is not None else 0.0
        ec = exec_confidence_from_spread(spread_bps)
        mode = classify_liquidation_mode(
            trigger_confidence=tc,
            heat=float(apex.heat_score),
            asymmetry=asym,
            atr_over_close=atr_over,
            degradation=apex.degradation,
        )
        upd.update(
            {
                "risk_asymmetry_score": asym,
                "risk_trigger_confidence": tc,
                "risk_execution_confidence": ec,
                "risk_heat_score": float(apex.heat_score),
                "risk_novelty_score": float(apex.novelty),
                "risk_reflexivity_score": float(apex.reflexivity_score),
                "risk_liquidation_mode": mode,
            }
        )
        # FB-CAN-031: deterministic false-positive / late-chase memory for auction penalty
        old_fp = float(getattr(risk, "trigger_false_positive_memory", 0.0) or 0.0)
        tr = trigger
        if tr is not None:
            inc = 0.0
            if tr.missed_move_flag:
                inc = max(inc, 0.35)
            elif tr.setup_valid and tr.pretrigger_valid and not tr.trigger_valid:
                inc = max(
                    inc,
                    _clip01(0.45 * float(tr.trigger_strength) + 0.25 * (1.0 - float(tr.trigger_confidence))),
                )
            decay = 0.82 if tr.trigger_valid else 0.96
            new_fp = _clip01(old_fp * decay + inc)
            upd["trigger_false_positive_memory"] = new_fp
    return risk.model_copy(update=upd)

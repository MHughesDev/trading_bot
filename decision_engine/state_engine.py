"""APEX canonical state / regime engine (FB-CAN-004).

Builds :class:`app.contracts.canonical_state.CanonicalStateOutput` from forecast packet + features.
Deterministic, replay-friendly; structural inputs default to neutral when absent from feature_row.
"""

from __future__ import annotations

from typing import Any

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
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


def build_canonical_state(
    pkt: ForecastPacket,
    feature_row: dict[str, float],
    *,
    spread_bps: float,
) -> CanonicalStateOutput:
    """Derive APEX canonical state from forecaster packet and microstructure features."""
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

    rc_inv = 1.0 - (max(p) - sorted(p)[-2] if len(p) > 1 else max(p))
    vt = rel_vol
    md = _clip01(abs(p_bull - p_bear))
    sd = _clip01(1.0 - max(p_bull + p_bear, p_side + p_vol))
    t_raw = 0.35 * rc_inv + 0.25 * vt + 0.2 * md + 0.2 * sd
    transition_probability = _clip01(t_raw)

    p_trend = _clip01(p_bull + p_bear)
    p_range = _clip01(p_side)
    p_stress = _clip01(p_vol * 0.55 + rel_vol * 0.25)
    p_dislocated = _clip01(p_vol * 0.2 + spread_stress * 0.35 + rel_vol * 0.2)
    p_transition = _clip01(transition_probability * 0.6 + (1.0 - max(p)) * 0.25)
    regime_probs = _normalize5([p_trend, p_range, p_stress, p_dislocated, p_transition])

    sorted_p = sorted(regime_probs, reverse=True)
    regime_confidence = _clip01((sorted_p[0] - sorted_p[1]) if len(sorted_p) > 1 else sorted_p[0])

    ood = _clip01(float(pkt.ood_score))
    novelty = _clip01(0.55 * ood + 0.45 * (1.0 - max(p)))

    rsi = _safe_float(feature_row, "rsi_14", 50.0)
    rsi_ext = _clip01(abs(rsi - 50.0) / 50.0)
    hf = rsi_ext * 0.2
    hl = rel_vol * 0.25
    ho = rel_vol * 0.15
    hx = spread_stress * 0.15
    hv = rel_vol * 0.2
    he = spread_stress * 0.15
    heat_raw = hf + hl + ho + hx + hv + he
    heat_score = _clip01(heat_raw)
    heat_components = {"Hf": hf, "Hl": hl, "Ho": ho, "Hx": hx, "Hv": hv, "He": he}

    reflexivity = _clip01(0.5 * rsi_ext + 0.35 * rel_vol + 0.15 * p_vol)

    deg = DegradationLevel.NORMAL
    if spread_bps > 75 or heat_score > 0.92 or transition_probability > 0.88:
        deg = DegradationLevel.NO_TRADE
    elif heat_score > 0.68 or transition_probability > 0.58 or novelty > 0.82:
        deg = DegradationLevel.DEFENSIVE
    elif heat_score > 0.45 or transition_probability > 0.38:
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
    )


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
) -> Any:
    """Attach canonical degradation, size multiplier, and FB-CAN-007 sizing inputs."""
    from app.contracts.risk import RiskState

    if not isinstance(risk, RiskState):
        return risk
    if apex is None:
        return risk

    upd: dict[str, Any] = {
        "canonical_degradation": apex.degradation,
        "canonical_size_multiplier": degradation_size_multiplier(apex.degradation),
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
                "risk_reflexivity_score": float(apex.reflexivity_score),
                "risk_liquidation_mode": mode,
            }
        )
    return risk.model_copy(update=upd)

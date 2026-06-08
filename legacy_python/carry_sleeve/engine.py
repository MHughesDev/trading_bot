"""Carry sleeve eligibility, sizing, and interaction with directional risk (FB-CAN-018)."""

from __future__ import annotations

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.carry_sleeve import CarrySleeveDecision
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.trigger import TriggerOutput
from carry_sleeve.config import CarrySleeveConfig


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


def funding_signal_from_features(feature_row: dict[str, float]) -> float:
    """Map funding fields to [0,1] extremity proxy (spec §8 funding domain)."""
    z = abs(_safe_float(feature_row, "funding_rate_zscore", 0.0))
    if z > 0:
        return _clip01(z / 4.0)
    fr = abs(_safe_float(feature_row, "funding_rate", 0.0))
    return _clip01(fr * 5000.0)


def evaluate_carry_sleeve(
    feature_row: dict[str, float],
    trigger: TriggerOutput,
    apex: CanonicalStateOutput,
    cfg: CarrySleeveConfig,
    *,
    directional_proposal: ActionProposal | None,
) -> CarrySleeveDecision:
    """
    Decide whether carry sleeve is active this tick and whether directional must be suppressed.

    Deterministic given inputs — suitable for replay event payloads.
    """
    reasons: list[str] = []
    if not cfg.carry_enabled:
        return CarrySleeveDecision(reason_codes=["carry_disabled"])

    fs = funding_signal_from_features(feature_row)
    tc = _clip01(float(trigger.trigger_confidence))
    quality = _clip01(fs * tc)

    if fs < cfg.carry_funding_threshold:
        return CarrySleeveDecision(
            eligible=False,
            funding_signal=fs,
            trigger_confidence=tc,
            decision_quality=quality,
            reason_codes=["funding_below_threshold"],
        )

    if apex.degradation == DegradationLevel.NO_TRADE:
        return CarrySleeveDecision(
            eligible=False,
            funding_signal=fs,
            trigger_confidence=tc,
            decision_quality=quality,
            reason_codes=["degradation_no_trade"],
        )

    reasons.append("funding_ok")
    eligible = True

    low_dir = float(trigger.trigger_confidence) < cfg.carry_low_directional_trigger_confidence
    neutral = directional_proposal is None or low_dir
    if cfg.carry_activation_requires_directional_neutrality and not neutral:
        return CarrySleeveDecision(
            eligible=eligible,
            active=False,
            funding_signal=fs,
            trigger_confidence=tc,
            decision_quality=quality,
            reason_codes=reasons + ["directional_not_neutral"],
        )

    cap = max(0.0, cfg.carry_max_exposure_usd * cfg.carry_independent_risk_multiplier)
    active = True
    reasons.append("carry_active")

    directional_blocked = bool(
        cfg.carry_attribution_isolation_required and active and directional_proposal is not None
    )
    if directional_blocked:
        reasons.append("isolation_suppress_directional")

    return CarrySleeveDecision(
        eligible=True,
        active=active,
        funding_signal=fs,
        trigger_confidence=tc,
        decision_quality=quality,
        target_notional_usd=cap,
        directional_blocked=directional_blocked,
        isolation_required=cfg.carry_attribution_isolation_required,
        reason_codes=reasons,
    )


def carry_direction_from_features(feature_row: dict[str, float]) -> int:
    z = _safe_float(feature_row, "funding_rate_zscore", 0.0)
    if abs(z) > 1e-12:
        return 1 if z > 0 else -1
    fr = _safe_float(feature_row, "funding_rate", 0.0)
    if abs(fr) > 1e-12:
        return 1 if fr > 0 else -1
    return 1


def build_carry_proposal(
    symbol: str,
    carry_dec: CarrySleeveDecision,
    *,
    feature_row: dict[str, float],
    max_per_symbol_usd: float,
) -> ActionProposal | None:
    """Express carry sleeve as a routed proposal for RiskEngine (isolated route)."""
    if not carry_dec.active or carry_dec.target_notional_usd <= 0:
        return None
    slot = max(max_per_symbol_usd, 1e-9)
    frac = min(1.0, carry_dec.target_notional_usd / slot)
    if frac <= 0:
        return None
    direction = carry_direction_from_features(feature_row)
    return ActionProposal(
        symbol=symbol,
        route_id=RouteId.CARRY,
        direction=direction if direction != 0 else 1,
        size_fraction=frac,
        stop_distance_pct=0.012,
        order_type="market",
        expiry_seconds=3_600,
    )

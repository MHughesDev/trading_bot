"""Canonical APEX decision sequence (FB-CAN-029).

Single place for the ordered stages shared by :meth:`DecisionPipeline.step` and documented as:

normalize (boundary input) → state → structure → trigger → auction → carry overlay → risk merge

``RiskEngine.evaluate`` and execution guidance live in :func:`decision_engine.run_step.run_decision_tick`
after the pipeline returns a proposal.
"""

from __future__ import annotations

from datetime import datetime
from decimal import Decimal

import numpy as np

from app.config.settings import AppSettings
from app.contracts.reason_codes import (
    PIP_BINDING_ABSTAIN,
    PIP_CARRY_SLEEVE_BLOCKED,
    PIP_NO_TRADE_SELECTED,
    normalize_reason_codes,
)
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.canonical_state import DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from app.contracts.structure_adapter import structure_from_forecast_packet
from carry_sleeve.config import CarrySleeveConfig
from carry_sleeve.engine import build_carry_proposal, evaluate_carry_sleeve
from decision_engine.spec_policy_proposal import run_spec_policy_step
from decision_engine.state_engine import (
    apply_normalization_degradation,
    build_canonical_state,
    classify_hard_override,
    merge_canonical_into_risk,
)
from decision_engine.trigger_engine import evaluate_trigger
from policy_model.system import PolicySystem


def regime_output_from_forecast_packet(pkt: ForecastPacket) -> RegimeOutput:
    """Map soft regime vector (length 4) to ``RegimeOutput`` for observability."""
    probs = list(pkt.regime_vector)
    if len(probs) < 4:
        probs = (probs + [0.25] * 4)[:4]
    s = sum(probs) or 1.0
    p = [x / s for x in probs[:4]]
    idx = int(np.argmax(p))
    sem_map = (SemanticRegime.BULL, SemanticRegime.BEAR, SemanticRegime.VOLATILE, SemanticRegime.SIDEWAYS)
    sem = sem_map[idx] if idx < 4 else SemanticRegime.SIDEWAYS
    return RegimeOutput(
        state_index=idx,
        semantic=sem,
        probabilities=p,
        confidence=float(max(p)),
    )


def run_canonical_decision_sequence_after_forecast(
    *,
    symbol: str,
    feature_effective: dict[str, float],
    spread_bps: float,
    mid_price: float,
    portfolio_equity_usd: float,
    position_signed_qty: Decimal | None,
    pkt: ForecastPacket,
    binding_abstain: bool,
    settings: AppSettings,
    risk: RiskState,
    policy_system: PolicySystem | None,
    feed_last_message_at: datetime | None = None,
    data_timestamp: datetime | None = None,
    now_ref: datetime | None = None,
    product_tradable: bool = True,
    current_total_exposure_usd: float = 0.0,
) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None, RiskState]:
    """Stages after ``ForecastPacket`` build: structure → state → trigger → auction → carry → ``RiskState``."""

    # --- structure (from forecast packet) ---
    canonical_structure = structure_from_forecast_packet(pkt)
    pkt.forecast_diagnostics["canonical_structure"] = canonical_structure.model_dump(mode="json")

    # --- state (canonical apex on regime output) ---
    regime_out = regime_output_from_forecast_packet(pkt)
    apex = build_canonical_state(
        pkt,
        feature_effective,
        spread_bps=spread_bps,
        settings=settings,
        structure=canonical_structure,
        data_timestamp=data_timestamp,
    )
    apex = apply_normalization_degradation(apex, feature_effective)
    ho, ho_kind = classify_hard_override(
        risk=risk,
        feature_row=feature_effective,
        spread_bps=spread_bps,
        settings=settings,
        feed_last_message_at=feed_last_message_at,
        data_timestamp=data_timestamp,
        now_ref=now_ref,
        product_tradable=product_tradable,
    )
    if ho:
        apex = apex.model_copy(update={"degradation": DegradationLevel.NO_TRADE})
    pkt.forecast_diagnostics["hard_override"] = {
        "active": ho,
        "kind": ho_kind.value,
    }
    pipe_codes: list[str] = []
    if ho:
        pipe_codes = normalize_reason_codes(
            ["pipeline_hard_override", f"override_{ho_kind.value}"]
            + list(getattr(apex, "safety_reason_codes", None) or [])
        )
    regime_out = regime_out.model_copy(
        update={
            "apex": apex,
            "canonical_regime_probabilities": list(apex.regime_probabilities),
            # Downstream should use 5-class vector + apex; keep HMM semantic for legacy charts
            "confidence": float(apex.regime_confidence),
        }
    )

    # --- trigger ---
    trig = evaluate_trigger(
        pkt,
        feature_effective,
        spread_bps=spread_bps,
        apex=apex,
        structure=canonical_structure,
        decision_timestamp=data_timestamp,
    )

    # --- merge canonical inputs into risk before auction (policy reads app_risk) ---
    risk = merge_canonical_into_risk(
        risk,
        apex,
        settings=settings,
        forecast_packet=pkt,
        trigger=trig,
        spread_bps=spread_bps,
        feature_row=feature_effective,
        hard_override_active=ho,
        hard_override_kind=ho_kind,
    )
    risk = risk.model_copy(update={"last_pipeline_no_trade_codes": list(pipe_codes)})

    mp = float(mid_price)
    eq = float(portfolio_equity_usd)

    if ho:
        fc = ForecastOutput(
            returns_1=0.0,
            returns_3=0.0,
            returns_5=0.0,
            returns_15=0.0,
            volatility=0.0,
            uncertainty=1.0,
        )
        route = RouteDecision(
            route_id=RouteId.NO_TRADE,
            confidence=0.0,
            ranking=[RouteId.NO_TRADE],
        )
        return regime_out, fc, route, None, risk

    if binding_abstain:
        fc = ForecastOutput(
            returns_1=0.0,
            returns_3=0.0,
            returns_5=0.0,
            returns_15=0.0,
            volatility=0.0,
            uncertainty=1.0,
        )
        route = RouteDecision(
            route_id=RouteId.NO_TRADE,
            confidence=0.0,
            ranking=[RouteId.NO_TRADE],
        )
        risk = risk.model_copy(
            update={"last_pipeline_no_trade_codes": [PIP_BINDING_ABSTAIN]},
        )
        return regime_out, fc, route, None, risk

    # --- auction (policy + opportunity auction inside run_spec_policy_step) ---
    fc, route, action = run_spec_policy_step(
        symbol,
        pkt,
        settings=settings,
        app_risk=risk,
        mid_price=mp,
        spread_bps=spread_bps,
        portfolio_equity_usd=eq,
        position_signed_qty=position_signed_qty,
        policy_system=policy_system,
        trigger=trig,
        apex=apex,
        feature_row=feature_effective,
        structure=canonical_structure,
        current_total_exposure_usd=current_total_exposure_usd,
    )

    # --- carry sleeve (overlays directional route when active) ---
    carry_cfg = CarrySleeveConfig.from_canonical_domains(settings.canonical.domains.carry)
    carry_dec = evaluate_carry_sleeve(
        feature_effective,
        trig,
        apex,
        carry_cfg,
        directional_proposal=action,
    )
    carry_prop: ActionProposal | None = None
    risk = risk.model_copy(
        update={"carry_sleeve_last": carry_dec.model_dump(mode="json")},
    )
    pkt.forecast_diagnostics["carry_sleeve"] = carry_dec.model_dump(mode="json")

    if carry_dec.active:
        if carry_dec.directional_blocked and action is not None:
            action = None
            route = RouteDecision(
                route_id=RouteId.NO_TRADE,
                confidence=0.0,
                ranking=[RouteId.NO_TRADE],
            )
            risk = risk.model_copy(
                update={
                    "last_pipeline_no_trade_codes": list(
                        risk.last_pipeline_no_trade_codes or []
                    )
                    + [PIP_CARRY_SLEEVE_BLOCKED]
                },
            )
        carry_prop = build_carry_proposal(
            symbol,
            carry_dec,
            feature_row=feature_effective,
            max_per_symbol_usd=float(settings.risk_max_per_symbol_usd),
        )
        if carry_prop is not None:
            action = carry_prop
            route = RouteDecision(
                route_id=RouteId.CARRY,
                confidence=min(1.0, carry_dec.funding_signal),
                ranking=[RouteId.CARRY, RouteId.NO_TRADE],
            )

    if (
        not pipe_codes
        and action is None
        and route.route_id == RouteId.NO_TRADE
        and not (carry_dec.active and carry_prop is not None)
    ):
        risk = risk.model_copy(
            update={"last_pipeline_no_trade_codes": [PIP_NO_TRADE_SELECTED]},
        )

    return regime_out, fc, route, action, risk


__all__ = [
    "regime_output_from_forecast_packet",
    "run_canonical_decision_sequence_after_forecast",
]

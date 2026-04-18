"""APEX canonical risk / sizing (FB-CAN-007).

Implements layered multipliers and constraints per APEX_UNIFIED master spec §11 and
auction/risk domain — without removing hard safety gates in RiskEngine.evaluate.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal

from pydantic import BaseModel, Field

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState


class RiskSizingDiagnostics(BaseModel):
    """Explainable breakdown of the last sizing pass (replay-friendly)."""

    base_notional_usd: float = 0.0
    after_degradation: float = 0.0
    after_inertia: float = 0.0
    after_asymmetry_boost: float = 0.0
    after_liquidation_mode: float = 0.0
    after_edge_budget: float = 0.0
    after_concentration: float = 0.0
    final_notional_usd: float = 0.0
    asymmetry_boost_applied: float = 1.0
    liquidation_mode: str = "neutral"
    reason_codes: list[str] = Field(default_factory=list)


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def exec_confidence_from_spread(spread_bps: float) -> float:
    spread_stress = _clip01(spread_bps / 80.0)
    return _clip01(1.0 - spread_stress * 0.8)


def classify_liquidation_mode(
    *,
    trigger_confidence: float,
    heat: float,
    asymmetry: float,
    atr_over_close: float,
    degradation: DegradationLevel | None,
) -> str:
    """Rough offense vs defense vs neutral for sizing multiplier."""
    if degradation in (DegradationLevel.DEFENSIVE, DegradationLevel.REDUCED) or heat > 0.72:
        return "defense"
    if trigger_confidence >= 0.35 and heat < 0.48 and asymmetry >= 0.38 and atr_over_close >= 0.0015:
        return "offense"
    return "neutral"


def liquidation_mode_multiplier(mode: str) -> float:
    return {"offense": 1.05, "defense": 0.88, "neutral": 1.0}.get(mode, 1.0)


def inertia_multiplier(
    *,
    direction: int,
    position_signed_qty: Decimal | None,
    mid_price: float,
    equity_usd: float,
) -> tuple[float, str | None]:
    """Reduce size on direction flip vs existing position (position inertia)."""
    pos = float(position_signed_qty) if position_signed_qty is not None else 0.0
    if abs(pos) < 1e-12 or direction == 0:
        return 1.0, None
    pos_side = 1 if pos > 0 else -1
    if pos_side == direction:
        return 1.0, None
    eq = max(equity_usd, 1e-9)
    pos_notional = abs(pos * float(mid_price))
    pos_frac = _clip01(pos_notional / eq)
    # Stronger reduction when flipping a large book
    m = max(0.35, 1.0 - 0.55 * pos_frac)
    return m, "position_inertia_flip"


def asymmetry_boost(
    *,
    asymmetry: float,
    trigger_confidence: float,
    execution_confidence: float,
    heat: float,
    reflexivity: float,
) -> tuple[float, str | None]:
    """Capped boost (max 1.2×) per master spec §11.3."""
    if (
        asymmetry > 0.55
        and trigger_confidence >= 0.22
        and execution_confidence >= 0.18
        and heat < 0.75
        and reflexivity < 0.82
    ):
        extra = min(0.2, max(0.0, (asymmetry - 0.5) * 0.85))
        return 1.0 + extra, "asymmetry_boost"
    return 1.0, None


def edge_budget_multiplier(
    *,
    heat: float,
    exposure_frac: float,
    symbol_exposure_frac: float,
) -> float:
    """Throttle when heat / concentration / overlap proxies are high."""
    proxy = _clip01(0.45 * heat + 0.35 * exposure_frac + 0.2 * symbol_exposure_frac)
    return max(0.35, 1.0 - 0.62 * proxy)


def concentration_multiplier(
    *,
    proposed_notional: float,
    symbol_existing_notional: float,
    equity_usd: float,
    total_exposure_usd: float,
    max_total_usd: float,
) -> tuple[float, str | None]:
    """Scale down when symbol or book concentration is high."""
    eq = max(equity_usd, 1e-9)
    sym_frac = (symbol_existing_notional + proposed_notional) / eq
    tot_frac = total_exposure_usd / max(max_total_usd, 1e-9)
    m = 1.0
    reason = None
    if sym_frac > 0.38:
        m *= max(0.25, 1.0 - (sym_frac - 0.38) * 2.2)
        reason = "symbol_concentration"
    if tot_frac > 0.82:
        m *= max(0.4, 1.0 - (tot_frac - 0.82) * 3.0)
        reason = "book_concentration" if reason is None else reason + "+book"
    return m, reason


@dataclass
class CanonicalNotionalResult:
    final_notional_usd: float
    diagnostics: RiskSizingDiagnostics


def compute_canonical_notional(
    proposal: ActionProposal,
    risk: RiskState,
    settings: AppSettings,
    *,
    mid_price: float,
    spread_bps: float,
    position_signed_qty: Decimal | None,
    current_total_exposure_usd: float,
    portfolio_equity_usd: float,
) -> CanonicalNotionalResult:
    """Layered sizing from proposal fraction → USD notional."""
    max_slot = float(settings.risk_max_per_symbol_usd)
    max_total = float(settings.risk_max_total_exposure_usd)
    eq = max(float(portfolio_equity_usd), 1e-9)

    base = proposal.size_fraction * max_slot
    diag = RiskSizingDiagnostics(base_notional_usd=base, reason_codes=[])

    deg_m = float(risk.canonical_size_multiplier)
    after_deg = base * deg_m
    diag.after_degradation = after_deg

    if proposal.route_id == RouteId.CARRY:
        carry_m = 0.55
        after_carry = after_deg * carry_m
        diag.reason_codes.append("carry_sleeve_independent_multiplier")
        inertia_m, inertia_reason = inertia_multiplier(
            direction=proposal.direction,
            position_signed_qty=position_signed_qty,
            mid_price=mid_price,
            equity_usd=eq,
        )
        after_in = after_carry * inertia_m
        diag.after_inertia = after_in
        if inertia_reason:
            diag.reason_codes.append(inertia_reason)
        asym = risk.risk_asymmetry_score
        tc = risk.risk_trigger_confidence
        ec = risk.risk_execution_confidence if risk.risk_execution_confidence is not None else exec_confidence_from_spread(
            spread_bps
        )
        heat = risk.risk_heat_score if risk.risk_heat_score is not None else 0.35
        refl = risk.risk_reflexivity_score if risk.risk_reflexivity_score is not None else 0.35
        boost_m = 1.0
        boost_reason: str | None = None
        if asym is not None and tc is not None:
            boost_m, boost_reason = asymmetry_boost(
                asymmetry=asym,
                trigger_confidence=tc,
                execution_confidence=ec,
                heat=heat,
                reflexivity=refl,
            )
        after_boost = after_in * min(boost_m, 1.15)
        diag.after_asymmetry_boost = after_boost
        diag.asymmetry_boost_applied = min(boost_m, 1.15)
        if boost_reason:
            diag.reason_codes.append(boost_reason)
        mode = risk.risk_liquidation_mode or "neutral"
        diag.liquidation_mode = mode
        liq_m = liquidation_mode_multiplier(mode)
        after_liq = after_boost * liq_m
        diag.after_liquidation_mode = after_liq
        pos = float(position_signed_qty) if position_signed_qty is not None else 0.0
        sym_existing = abs(pos * float(mid_price))
        exposure_frac = _clip01(current_total_exposure_usd / max_total) if max_total > 0 else 0.0
        sym_exposure_frac = _clip01(sym_existing / eq)
        edge_m = edge_budget_multiplier(
            heat=heat,
            exposure_frac=exposure_frac,
            symbol_exposure_frac=sym_exposure_frac,
        )
        after_edge = after_liq * edge_m
        diag.after_edge_budget = after_edge
        if edge_m < 0.999:
            diag.reason_codes.append("edge_budget")
        conc_m, conc_reason = concentration_multiplier(
            proposed_notional=after_edge,
            symbol_existing_notional=sym_existing,
            equity_usd=eq,
            total_exposure_usd=current_total_exposure_usd,
            max_total_usd=max_total,
        )
        final = after_edge * conc_m
        diag.after_concentration = final
        if conc_reason:
            diag.reason_codes.append(conc_reason)
        final = min(final, max_slot)
        if current_total_exposure_usd + final > max_total:
            headroom = max(0.0, max_total - current_total_exposure_usd)
            final = min(final, headroom)
            diag.reason_codes.append("total_exposure_cap")
        diag.final_notional_usd = final
        return CanonicalNotionalResult(final_notional_usd=final, diagnostics=diag)

    inertia_m, inertia_reason = inertia_multiplier(
        direction=proposal.direction,
        position_signed_qty=position_signed_qty,
        mid_price=mid_price,
        equity_usd=eq,
    )
    after_in = after_deg * inertia_m
    diag.after_inertia = after_in
    if inertia_reason:
        diag.reason_codes.append(inertia_reason)

    asym = risk.risk_asymmetry_score
    tc = risk.risk_trigger_confidence
    ec = risk.risk_execution_confidence if risk.risk_execution_confidence is not None else exec_confidence_from_spread(
        spread_bps
    )
    heat = risk.risk_heat_score if risk.risk_heat_score is not None else 0.35
    refl = risk.risk_reflexivity_score if risk.risk_reflexivity_score is not None else 0.35

    boost_m = 1.0
    boost_reason: str | None = None
    if asym is not None and tc is not None:
        boost_m, boost_reason = asymmetry_boost(
            asymmetry=asym,
            trigger_confidence=tc,
            execution_confidence=ec,
            heat=heat,
            reflexivity=refl,
        )
    after_boost = after_in * min(boost_m, 1.2)
    diag.after_asymmetry_boost = after_boost
    diag.asymmetry_boost_applied = min(boost_m, 1.2)
    if boost_reason:
        diag.reason_codes.append(boost_reason)

    mode = risk.risk_liquidation_mode or "neutral"
    diag.liquidation_mode = mode
    liq_m = liquidation_mode_multiplier(mode)
    after_liq = after_boost * liq_m
    diag.after_liquidation_mode = after_liq

    pos = float(position_signed_qty) if position_signed_qty is not None else 0.0
    sym_existing = abs(pos * float(mid_price))
    exposure_frac = _clip01(current_total_exposure_usd / max_total) if max_total > 0 else 0.0
    sym_exposure_frac = _clip01(sym_existing / eq)
    edge_m = edge_budget_multiplier(
        heat=heat,
        exposure_frac=exposure_frac,
        symbol_exposure_frac=sym_exposure_frac,
    )
    after_edge = after_liq * edge_m
    diag.after_edge_budget = after_edge
    if edge_m < 0.999:
        diag.reason_codes.append("edge_budget")

    conc_m, conc_reason = concentration_multiplier(
        proposed_notional=after_edge,
        symbol_existing_notional=sym_existing,
        equity_usd=eq,
        total_exposure_usd=current_total_exposure_usd,
        max_total_usd=max_total,
    )
    final = after_edge * conc_m
    diag.after_concentration = final
    if conc_reason:
        diag.reason_codes.append(conc_reason)

    final = min(final, max_slot)
    if current_total_exposure_usd + final > max_total:
        headroom = max(0.0, max_total - current_total_exposure_usd)
        final = min(final, headroom)
        diag.reason_codes.append("total_exposure_cap")

    diag.final_notional_usd = final
    return CanonicalNotionalResult(final_notional_usd=final, diagnostics=diag)

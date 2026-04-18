"""APEX canonical risk / sizing (FB-CAN-007, FB-CAN-045).

Implements layered multipliers and constraints per APEX_UNIFIED master spec §11 and
auction/risk domain — without removing hard safety gates in RiskEngine.evaluate.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Any

from pydantic import BaseModel, Field

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.decisions import ActionProposal, RouteId
from app.contracts.risk import RiskState


def _risk_sizing_domain(settings: AppSettings) -> dict[str, Any]:
    try:
        rs = settings.canonical.domains.risk_sizing
        return dict(rs) if isinstance(rs, dict) else {}
    except Exception:
        return {}


def _cfg_float(dom: dict[str, Any], key: str, default: float) -> float:
    v = dom.get(key)
    if v is None:
        return default
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


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
    asymmetry_boost_raw: float = 1.0
    liquidation_mode: str = "neutral"
    # FB-CAN-045 — transparent intermediate multipliers
    degradation_base_multiplier: float = 1.0
    transition_multiplier: float = 1.0
    novelty_multiplier: float = 1.0
    composite_degradation_multiplier: float = 1.0
    inertia_multiplier: float = 1.0
    liquidation_mode_multiplier: float = 1.0
    edge_budget_multiplier: float = 1.0
    # FB-CAN-076 — headroom = multiplier; stress = 1 - headroom (monitoring proxy)
    edge_budget_headroom: float = 1.0
    edge_budget_stress: float = 0.0
    concentration_multiplier: float = 1.0
    symbol_concentration_multiplier: float = 1.0
    book_concentration_multiplier: float = 1.0
    reason_codes: list[str] = Field(default_factory=list)
    config_snapshot: dict[str, float] = Field(default_factory=dict)


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
    penalty_weight: float = 0.55,
    min_multiplier: float = 0.35,
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
    w = max(0.0, float(penalty_weight))
    floor = max(0.0, min(1.0, float(min_multiplier)))
    m = max(floor, 1.0 - w * pos_frac)
    return m, "position_inertia_flip"


def asymmetry_boost(
    *,
    asymmetry: float,
    trigger_confidence: float,
    execution_confidence: float,
    heat: float,
    reflexivity: float,
    boost_cap: float = 1.2,
) -> tuple[float, str | None]:
    """Capped boost (default max 1.2×) per master spec §11.3."""
    cap = max(1.0, min(2.0, float(boost_cap)))
    max_extra = cap - 1.0
    if (
        asymmetry > 0.55
        and trigger_confidence >= 0.22
        and execution_confidence >= 0.18
        and heat < 0.75
        and reflexivity < 0.82
    ):
        extra = min(max_extra, max(0.0, (asymmetry - 0.5) * 0.85))
        return 1.0 + extra, "asymmetry_boost"
    return 1.0, None


def edge_budget_multiplier(
    *,
    heat: float,
    exposure_frac: float,
    symbol_exposure_frac: float,
    weight_heat: float = 0.45,
    weight_exposure: float = 0.35,
    weight_symbol: float = 0.2,
    strength: float = 0.62,
    min_mult: float = 0.35,
) -> float:
    """Throttle when heat / concentration / overlap proxies are high."""
    wh = max(0.0, float(weight_heat))
    we = max(0.0, float(weight_exposure))
    ws = max(0.0, float(weight_symbol))
    wsum = wh + we + ws
    if wsum <= 0.0:
        return 1.0
    h = _clip01(heat)
    e = _clip01(exposure_frac)
    sy = _clip01(symbol_exposure_frac)
    proxy = _clip01((wh * h + we * e + ws * sy) / wsum)
    k = max(0.0, min(1.0, float(strength)))
    floor = max(0.0, min(1.0, float(min_mult)))
    return max(floor, 1.0 - k * proxy)


def concentration_multiplier(
    *,
    proposed_notional: float,
    symbol_existing_notional: float,
    equity_usd: float,
    total_exposure_usd: float,
    max_total_usd: float,
    sym_threshold: float = 0.38,
    sym_strength: float = 2.2,
    sym_floor: float = 0.25,
    book_threshold: float = 0.82,
    book_strength: float = 3.0,
    book_floor: float = 0.4,
) -> tuple[float, float, float, str | None]:
    """Scale down when symbol or book concentration is high. Returns (combined, sym_m, book_m, reason)."""
    eq = max(equity_usd, 1e-9)
    sym_frac = (symbol_existing_notional + proposed_notional) / eq
    tot_frac = total_exposure_usd / max(max_total_usd, 1e-9)
    sym_m = 1.0
    book_m = 1.0
    reason = None
    st = float(sym_threshold)
    if sym_frac > st:
        sym_m = max(sym_floor, 1.0 - (sym_frac - st) * sym_strength)
        reason = "symbol_concentration"
    bt = float(book_threshold)
    if tot_frac > bt:
        book_m = max(book_floor, 1.0 - (tot_frac - bt) * book_strength)
        reason = "book_concentration" if reason is None else reason + "+book"
    return sym_m * book_m, sym_m, book_m, reason


@dataclass
class CanonicalNotionalResult:
    final_notional_usd: float
    diagnostics: RiskSizingDiagnostics


def _apply_degradation_terms_to_diag(
    diag: RiskSizingDiagnostics,
    *,
    composite_m: float,
    risk: RiskState,
) -> None:
    """Fill degradation-related fields from RiskState (set in merge_canonical_into_risk)."""
    comp = float(composite_m)
    diag.composite_degradation_multiplier = comp
    terms = getattr(risk, "canonical_degradation_sizing_terms", None)
    if isinstance(terms, dict):
        db = terms.get("degradation_base_multiplier")
        tm = terms.get("transition_multiplier")
        nm = terms.get("novelty_multiplier")
        if db is not None:
            try:
                diag.degradation_base_multiplier = float(db)
            except (TypeError, ValueError):
                diag.degradation_base_multiplier = comp
        else:
            diag.degradation_base_multiplier = comp
        if tm is not None:
            try:
                diag.transition_multiplier = float(tm)
            except (TypeError, ValueError):
                diag.transition_multiplier = 1.0
        else:
            diag.transition_multiplier = 1.0
        if nm is not None:
            try:
                diag.novelty_multiplier = float(nm)
            except (TypeError, ValueError):
                diag.novelty_multiplier = 1.0
        else:
            diag.novelty_multiplier = 1.0
    else:
        diag.degradation_base_multiplier = comp
        diag.transition_multiplier = 1.0
        diag.novelty_multiplier = 1.0


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
    dom = _risk_sizing_domain(settings)
    q_cap = _cfg_float(dom, "quantile_asymmetry_boost_cap", 1.2)
    carry_cap = _cfg_float(dom, "carry_asymmetry_boost_cap", 1.15)
    in_w = _cfg_float(dom, "position_inertia_penalty_weight", 0.55)
    in_min = _cfg_float(dom, "position_inertia_min_multiplier", 0.35)

    max_slot = float(settings.risk_max_per_symbol_usd)
    max_total = float(settings.risk_max_total_exposure_usd)
    eq = max(float(portfolio_equity_usd), 1e-9)

    base = proposal.size_fraction * max_slot
    diag = RiskSizingDiagnostics(base_notional_usd=base, reason_codes=[])
    comp_deg = float(risk.canonical_size_multiplier)
    _apply_degradation_terms_to_diag(diag, composite_m=comp_deg, risk=risk)
    after_deg = base * comp_deg
    diag.after_degradation = after_deg

    snap_keys = (
        "quantile_asymmetry_boost_cap",
        "carry_asymmetry_boost_cap",
        "position_inertia_penalty_weight",
        "edge_budget_weight_heat",
        "edge_budget_strength",
        "symbol_concentration_threshold",
        "book_concentration_threshold",
    )
    diag.config_snapshot = {k: _cfg_float(dom, k, 0.0) for k in snap_keys if k in dom}
    if not diag.config_snapshot:
        diag.config_snapshot = {
            "quantile_asymmetry_boost_cap": q_cap,
            "carry_asymmetry_boost_cap": carry_cap,
            "position_inertia_penalty_weight": in_w,
        }

    if proposal.route_id == RouteId.CARRY:
        carry_m = 0.55
        after_carry = after_deg * carry_m
        diag.reason_codes.append("carry_sleeve_independent_multiplier")
        inertia_m, inertia_reason = inertia_multiplier(
            direction=proposal.direction,
            position_signed_qty=position_signed_qty,
            mid_price=mid_price,
            equity_usd=eq,
            penalty_weight=in_w,
            min_multiplier=in_min,
        )
        diag.inertia_multiplier = inertia_m
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
                boost_cap=carry_cap,
            )
        diag.asymmetry_boost_raw = boost_m
        cap_use = min(boost_m, carry_cap)
        after_boost = after_in * cap_use
        diag.after_asymmetry_boost = after_boost
        diag.asymmetry_boost_applied = cap_use
        if boost_reason:
            diag.reason_codes.append(boost_reason)
        mode = risk.risk_liquidation_mode or "neutral"
        diag.liquidation_mode = mode
        liq_m = liquidation_mode_multiplier(mode)
        diag.liquidation_mode_multiplier = liq_m
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
            weight_heat=_cfg_float(dom, "edge_budget_weight_heat", 0.45),
            weight_exposure=_cfg_float(dom, "edge_budget_weight_exposure", 0.35),
            weight_symbol=_cfg_float(dom, "edge_budget_weight_symbol_exposure", 0.2),
            strength=_cfg_float(dom, "edge_budget_strength", 0.62),
            min_mult=_cfg_float(dom, "edge_budget_min_multiplier", 0.35),
        )
        diag.edge_budget_multiplier = edge_m
        diag.edge_budget_headroom = float(edge_m)
        diag.edge_budget_stress = _clip01(1.0 - float(edge_m))
        after_edge = after_liq * edge_m
        diag.after_edge_budget = after_edge
        if edge_m < 0.999:
            diag.reason_codes.append("edge_budget")
        conc_m, sym_part, book_part, conc_reason = concentration_multiplier(
            proposed_notional=after_edge,
            symbol_existing_notional=sym_existing,
            equity_usd=eq,
            total_exposure_usd=current_total_exposure_usd,
            max_total_usd=max_total,
            sym_threshold=_cfg_float(dom, "symbol_concentration_threshold", 0.38),
            sym_strength=_cfg_float(dom, "symbol_concentration_strength", 2.2),
            sym_floor=_cfg_float(dom, "symbol_concentration_min_floor", 0.25),
            book_threshold=_cfg_float(dom, "book_concentration_threshold", 0.82),
            book_strength=_cfg_float(dom, "book_concentration_strength", 3.0),
            book_floor=_cfg_float(dom, "book_concentration_min_floor", 0.4),
        )
        diag.concentration_multiplier = conc_m
        diag.symbol_concentration_multiplier = sym_part
        diag.book_concentration_multiplier = book_part
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
        penalty_weight=in_w,
        min_multiplier=in_min,
    )
    diag.inertia_multiplier = inertia_m
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
            boost_cap=q_cap,
        )
    diag.asymmetry_boost_raw = boost_m
    cap_use = min(boost_m, q_cap)
    after_boost = after_in * cap_use
    diag.after_asymmetry_boost = after_boost
    diag.asymmetry_boost_applied = cap_use
    if boost_reason:
        diag.reason_codes.append(boost_reason)

    mode = risk.risk_liquidation_mode or "neutral"
    diag.liquidation_mode = mode
    liq_m = liquidation_mode_multiplier(mode)
    diag.liquidation_mode_multiplier = liq_m
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
        weight_heat=_cfg_float(dom, "edge_budget_weight_heat", 0.45),
        weight_exposure=_cfg_float(dom, "edge_budget_weight_exposure", 0.35),
        weight_symbol=_cfg_float(dom, "edge_budget_weight_symbol_exposure", 0.2),
        strength=_cfg_float(dom, "edge_budget_strength", 0.62),
        min_mult=_cfg_float(dom, "edge_budget_min_multiplier", 0.35),
    )
    diag.edge_budget_multiplier = edge_m
    diag.edge_budget_headroom = float(edge_m)
    diag.edge_budget_stress = _clip01(1.0 - float(edge_m))
    after_edge = after_liq * edge_m
    diag.after_edge_budget = after_edge
    if edge_m < 0.999:
        diag.reason_codes.append("edge_budget")

    conc_m, sym_part, book_part, conc_reason = concentration_multiplier(
        proposed_notional=after_edge,
        symbol_existing_notional=sym_existing,
        equity_usd=eq,
        total_exposure_usd=current_total_exposure_usd,
        max_total_usd=max_total,
        sym_threshold=_cfg_float(dom, "symbol_concentration_threshold", 0.38),
        sym_strength=_cfg_float(dom, "symbol_concentration_strength", 2.2),
        sym_floor=_cfg_float(dom, "symbol_concentration_min_floor", 0.25),
        book_threshold=_cfg_float(dom, "book_concentration_threshold", 0.82),
        book_strength=_cfg_float(dom, "book_concentration_strength", 3.0),
        book_floor=_cfg_float(dom, "book_concentration_min_floor", 0.4),
    )
    diag.concentration_multiplier = conc_m
    diag.symbol_concentration_multiplier = sym_part
    diag.book_concentration_multiplier = book_part
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

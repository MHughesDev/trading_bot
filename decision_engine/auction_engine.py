"""APEX opportunity auction — scoring, penalties, top-N (FB-CAN-006).

Canonical formula: APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md §5–9.
Single-symbol path ranks long vs short vs flat using the same score machinery.
"""

from __future__ import annotations

from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.auction import AuctionCandidateRecord, AuctionResult
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.decisions import ActionProposal
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from app.contracts.trigger import TriggerOutput
from decision_engine.trigger_engine import asymmetry_score, state_alignment_score


def _clip(x: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, float(x)))


def _structural_confidence(pkt: ForecastPacket) -> float:
    cs = pkt.confidence_score
    if isinstance(cs, list):
        return _clip(sum(cs) / max(len(cs), 1), 0.0, 1.0)
    return _clip(float(cs), 0.0, 1.0)


def _exec_confidence(spread_bps: float) -> float:
    spread_stress = _clip(spread_bps / 80.0, 0.0, 1.0)
    return _clip(1.0 - spread_stress * 0.8, 0.0, 1.0)


def _degradation_penalty(deg: DegradationLevel | None) -> float:
    if deg is None or deg == DegradationLevel.NORMAL:
        return 0.0
    if deg == DegradationLevel.REDUCED:
        return 0.15
    if deg == DegradationLevel.DEFENSIVE:
        return 0.45
    return 1.0


def _oi_liquidation_proxy(pkt: ForecastPacket, feature_row: dict[str, float]) -> tuple[float, float]:
    """O, L proxies from interval width and relative vol when OI fields absent."""
    w0 = float(pkt.interval_width[0]) if pkt.interval_width else 0.0
    close = max(abs(float(feature_row.get("close", 1.0))), 1e-12)
    atr = float(feature_row.get("atr_14", 0.0) or 0.0)
    rel = _clip(atr / close * 50.0, 0.0, 1.0)
    oi_struct = _clip(w0 * 3.0, 0.0, 1.0)
    liq_opp = _clip(rel * 0.6 + w0 * 2.0, 0.0, 1.0)
    return oi_struct, liq_opp


def _directional_asymmetry(pkt: ForecastPacket, direction: int) -> float:
    """Asymmetry aligned with proposed direction (+1 long, -1 short)."""
    base = asymmetry_score(pkt)
    if not pkt.q_low or not pkt.q_high or not pkt.q_med:
        return base
    lo, hi, med = float(pkt.q_low[0]), float(pkt.q_high[0]), float(pkt.q_med[0])
    width = max(hi - lo, 1e-12)
    skew = ((med - lo) / width - 0.5) * 2.0
    if direction > 0:
        return _clip(base * (0.5 + 0.5 * max(0.0, skew)), 0.0, 1.0)
    if direction < 0:
        return _clip(base * (0.5 + 0.5 * max(0.0, -skew)), 0.0, 1.0)
    return base


def run_opportunity_auction(
    symbol: str,
    forecast_packet: ForecastPacket,
    *,
    apex: CanonicalStateOutput,
    trigger: TriggerOutput,
    app_risk: RiskState,
    spread_bps: float,
    feature_row: dict[str, float],
    settings: AppSettings,
    portfolio_equity_usd: float,
    position_signed_qty: Decimal | None,
    base_proposal: ActionProposal | None,
    top_n: int = 1,
) -> tuple[ActionProposal | None, AuctionResult]:
    """
    Build long/short/flat candidates, score, select top-N under notional cap.

    When ``base_proposal`` is None, only flat is considered (still logs auction).
    """
    exec_conf = _exec_confidence(spread_bps)
    S_align = state_alignment_score(apex)
    C_conf = _clip(0.5 * apex.regime_confidence + 0.5 * _structural_confidence(forecast_packet), 0.0, 1.0)
    T_trig = _clip(0.5 * trigger.trigger_strength + 0.5 * trigger.trigger_confidence, 0.0, 1.0)
    O_oi, L_liq = _oi_liquidation_proxy(forecast_packet, feature_row)

    eq = max(float(portfolio_equity_usd), 1e-9)
    qty = float(position_signed_qty) if position_signed_qty is not None else 0.0
    mp = float(feature_row.get("close", 1.0))
    pos_frac = abs(qty * mp) / eq
    heat = apex.heat_score
    deg = app_risk.canonical_degradation or apex.degradation

    max_per = float(settings.risk_max_per_symbol_usd)
    max_notional = top_n * max_per

    wA, wS, wC, wT, wE, wO, wL = 0.18, 0.14, 0.18, 0.16, 0.14, 0.1, 0.1
    wD, wM, wP, wG, wB, wR = 0.35, 0.2, 0.15, 0.4, 0.35, 0.35

    candidates: list[tuple[int, ActionProposal | None]] = []
    if base_proposal is not None:
        long_p = base_proposal.model_copy(update={"direction": 1})
        short_p = base_proposal.model_copy(update={"direction": -1})
        candidates.append((1, long_p))
        candidates.append((-1, short_p))
    candidates.append((0, None))

    min_trig_conf = 0.08
    min_decision_conf = 0.12
    min_exec_conf = 0.08

    records: list[AuctionCandidateRecord] = []
    scored: list[tuple[float, int, ActionProposal | None, AuctionCandidateRecord]] = []

    for direction, proposal in candidates:
        reasons: list[str] = []
        eligible = True

        if direction != 0 and proposal is None:
            continue

        if direction == 0:
            eligible = True
        else:
            if deg == DegradationLevel.NO_TRADE:
                eligible = False
                reasons.append("degradation_no_trade")
            if trigger.missed_move_flag:
                eligible = False
                reasons.append("missed_move")
            if not trigger.trigger_valid:
                eligible = False
                reasons.append("trigger_invalid")
            if trigger.trigger_confidence < min_trig_conf:
                eligible = False
                reasons.append("trigger_confidence_below_min")
            if C_conf < min_decision_conf:
                eligible = False
                reasons.append("decision_confidence_below_min")
            if exec_conf < min_exec_conf:
                eligible = False
                reasons.append("execution_confidence_below_min")

        A = _directional_asymmetry(forecast_packet, direction) if direction != 0 else asymmetry_score(
            forecast_packet
        )
        D_div = _clip(heat * 0.4 + pos_frac * 0.5, 0.0, 1.0)
        M_overlap = _clip(float(forecast_packet.ood_score), 0.0, 1.0)
        P_fp = _clip(1.0 - apex.regime_confidence, 0.0, 1.0) * 0.3
        G_deg = _degradation_penalty(deg)
        Cq = pos_frac
        Ov = _clip(abs(float(forecast_packet.ood_score)) * 0.5 + heat * 0.3, 0.0, 1.0)
        N_deploy = _clip(pos_frac * float(app_risk.canonical_size_multiplier), 0.0, 1.0)
        B_edge = _clip(0.35 * heat + 0.3 * Cq + 0.2 * Ov + 0.15 * N_deploy, 0.0, 1.0)
        R_conc = _clip(pos_frac * 0.7 + heat * 0.25, 0.0, 1.0)

        raw = (
            wA * A
            + wS * S_align
            + wC * C_conf
            + wT * T_trig
            + wE * exec_conf
            + wO * O_oi
            + wL * L_liq
            - wD * D_div
            - wM * M_overlap
            - wP * P_fp
            - wG * G_deg
            - wB * B_edge
            - wR * R_conc
        )
        if direction == 0:
            raw = raw * 0.25 - 0.15

        auction_score = _clip(raw, -1.0, 1.0)

        comps = {
            "A": A,
            "S": S_align,
            "C": C_conf,
            "T": T_trig,
            "E": exec_conf,
            "O": O_oi,
            "L": L_liq,
        }
        pens = {
            "D": D_div,
            "M": M_overlap,
            "P": P_fp,
            "G": G_deg,
            "B": B_edge,
            "R": R_conc,
        }

        rec = AuctionCandidateRecord(
            symbol=symbol,
            direction=direction,
            eligible=eligible,
            status="rejected" if not eligible else "pending",
            auction_score=auction_score,
            components=comps,
            penalties=pens,
            reasons=reasons.copy(),
        )
        records.append(rec)
        if eligible:
            scored.append((auction_score, direction, proposal, rec))

    scored.sort(key=lambda x: (-x[0], -trigger.trigger_confidence, -exec_conf, -x[1]))

    winner: ActionProposal | None = None
    sel_score: float | None = None
    sel_dir: int | None = None
    total_notional = 0.0
    pick_count = 0

    for auction_score, direction, proposal, rec in scored:
        if direction == 0:
            rec.status = "selected"
            winner = None
            sel_score = auction_score
            sel_dir = 0
            pick_count = 1
            break
        if proposal is None:
            continue
        notion = proposal.size_fraction * max_per
        if total_notional + notion > max_notional + 1e-9:
            rec.status = "suppressed"
            rec.reasons.append("notional_budget")
            continue
        if pick_count >= top_n:
            rec.status = "suppressed"
            rec.reasons.append("top_n_limit")
            continue
        rec.status = "selected"
        winner = proposal
        sel_score = auction_score
        sel_dir = direction
        total_notional += notion
        pick_count += 1
        break

    for rec in records:
        if rec.status == "pending":
            rec.status = "suppressed"
            rec.reasons.append("outranked")

    result = AuctionResult(
        selected_symbol=symbol if winner is not None else None,
        selected_direction=sel_dir,
        selected_score=sel_score,
        records=records,
        top_n_limit=top_n,
        max_notional_usd=max_notional,
        selected_notional_usd=total_notional,
    )
    return winner, result

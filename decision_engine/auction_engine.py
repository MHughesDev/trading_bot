"""APEX opportunity auction — scoring, penalties, top-N (FB-CAN-006).

Canonical formula: APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md §5–9.
Single-symbol path ranks long vs short vs flat using the same score machinery.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Any

from app.config.settings import AppSettings
from app.contracts.auction import AuctionCandidateRecord, AuctionResult
from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from app.contracts.canonical_structure import CanonicalStructureOutput
from app.contracts.decisions import ActionProposal
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from app.contracts.structure_adapter import structure_from_forecast_packet
from app.contracts.trigger import TriggerOutput
from decision_engine.trigger_engine import state_alignment_score


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


def _thesis_bucket_key(
    apex: CanonicalStateOutput,
    st: CanonicalStructureOutput,
    direction: int,
) -> str:
    """Deterministic thesis bucket for clustering / caps (FB-CAN-034)."""
    rp = list(apex.regime_probabilities)
    if len(rp) >= 5:
        idx = int(max(range(min(5, len(rp))), key=lambda i: rp[i]))
    else:
        idx = 0
    regime_names = ("trend", "range", "stress", "dislocated", "transition")
    rname = regime_names[idx] if idx < 5 else "unknown"
    bias = "long" if direction > 0 else "short" if direction < 0 else "flat"
    cp = float(st.continuation_probability)
    c_bucket = "hi" if cp >= 0.55 else "lo"
    return f"{rname}:{bias}:{c_bucket}"


def _liq_cluster_key(feature_row: dict[str, float], pkt: ForecastPacket) -> str:
    """Quantized liquidation-geometry bucket for overlap penalties (FB-CAN-034)."""
    w0 = float(pkt.interval_width[0]) if pkt.interval_width else 0.0
    close = max(abs(float(feature_row.get("close", 1.0))), 1e-12)
    atr = float(feature_row.get("atr_14", 0.0) or 0.0)
    rel = _clip(atr / close * 50.0, 0.0, 1.0)
    liq = _clip(rel * 0.6 + w0 * 2.0, 0.0, 1.0)
    bin_idx = min(4, int(liq * 5.0))
    return f"L{bin_idx}"


def _corr_candidate_vs_book(
    direction: int,
    pos_frac: float,
    apex: CanonicalStateOutput,
    st: CanonicalStructureOutput,
) -> float:
    """Proxy for candidate-to-book correlation / same-side concentration."""
    qty_sign = 1.0 if pos_frac > 1e-12 else (-1.0 if pos_frac < -1e-12 else 0.0)
    if direction == 0:
        return _clip(abs(pos_frac) * 0.6 + 0.1 * float(st.model_correlation_penalty), 0.0, 1.0)
    align = float(direction) * qty_sign
    if align > 0:
        return _clip(
            abs(pos_frac) + 0.12 * float(apex.heat_score) + 0.08 * float(st.model_correlation_penalty),
            0.0,
            1.0,
        )
    if align < 0:
        return _clip(abs(pos_frac) * 0.55 + 0.15 * float(st.model_correlation_penalty), 0.0, 1.0)
    return _clip(abs(pos_frac) * 0.35 + 0.1 * float(st.model_correlation_penalty), 0.0, 1.0)


def _corr_long_vs_short(st: CanonicalStructureOutput) -> float:
    """Structural coupling between +1 and -1 candidates (model co-movement)."""
    return _clip(0.35 + 0.65 * float(st.model_correlation_penalty), 0.0, 1.0)


def _directional_asymmetry(
    pkt: ForecastPacket,
    direction: int,
    structure: CanonicalStructureOutput | None = None,
) -> float:
    """Asymmetry aligned with proposed direction (+1 long, -1 short)."""
    st = structure if structure is not None else structure_from_forecast_packet(pkt)
    base = float(st.asymmetry_score)
    skew = float(st.directional_bias)
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
    structure: CanonicalStructureOutput | None = None,
) -> tuple[ActionProposal | None, AuctionResult]:
    """
    Build long/short/flat candidates, score, select top-N under notional cap.

    When ``base_proposal`` is None, only flat is considered (still logs auction).
    """
    st = structure if structure is not None else structure_from_forecast_packet(forecast_packet)
    exec_conf = _exec_confidence(spread_bps)
    dq_pen = _clip(float(feature_row.get("canonical_exec_quality_penalty", 0.0)), 0.0, 1.0)
    exec_conf = _clip(exec_conf * (1.0 - 0.45 * dq_pen), 0.0, 1.0)
    S_align = state_alignment_score(apex)
    C_conf = _clip(
        0.45 * apex.regime_confidence
        + 0.35 * float(st.model_agreement_score)
        + 0.2 * _structural_confidence(forecast_packet),
        0.0,
        1.0,
    )
    T_trig = _clip(0.5 * trigger.trigger_strength + 0.5 * trigger.trigger_confidence, 0.0, 1.0)
    O_oi, L_liq = _oi_liquidation_proxy(forecast_packet, feature_row)

    eq = max(float(portfolio_equity_usd), 1e-9)
    qty = float(position_signed_qty) if position_signed_qty is not None else 0.0
    mp = float(feature_row.get("close", 1.0))
    signed_pos_frac = (qty * mp) / eq
    pos_frac = abs(signed_pos_frac)
    heat = apex.heat_score
    deg = app_risk.canonical_degradation or apex.degradation

    max_per = float(settings.risk_max_per_symbol_usd)
    max_notional = top_n * max_per

    wA, wS, wC, wT, wE, wO, wL = 0.18, 0.14, 0.18, 0.16, 0.14, 0.1, 0.1
    wD, wM, wP, wG, wB, wR = 0.35, 0.2, 0.15, 0.4, 0.35, 0.35
    d1, d2, d3 = 0.45, 0.35, 0.2
    corr_ls = _corr_long_vs_short(st)
    liq_cluster_key = _liq_cluster_key(feature_row, forecast_packet)

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
    thesis_keys: dict[str, str] = {}

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

        A = (
            _directional_asymmetry(forecast_packet, direction, structure=st)
            if direction != 0
            else float(st.asymmetry_score)
        )
        tkey = _thesis_bucket_key(apex, st, direction)
        thesis_keys[str(int(direction))] = tkey

        D_corr = _corr_candidate_vs_book(direction, signed_pos_frac, apex, st)
        if direction == 1 or direction == -1:
            D_corr = max(D_corr, 0.55 * corr_ls)
        D_thesis = _clip(
            0.38 * float(apex.novelty)
            + 0.32 * float(st.fragility_score)
            + 0.3 * abs(float(forecast_packet.ood_score)),
            0.0,
            1.0,
        )
        D_liq = _clip(
            0.55 * L_liq + 0.28 * heat + 0.17 * float(forecast_packet.ood_score),
            0.0,
            1.0,
        )
        D_div = _clip(d1 * D_corr + d2 * D_thesis + d3 * D_liq, 0.0, 1.0)
        M_overlap = _clip(
            0.5 * float(forecast_packet.ood_score) + 0.5 * float(st.fragility_score),
            0.0,
            1.0,
        )
        rc_pen = _clip(1.0 - apex.regime_confidence, 0.0, 1.0) * 0.3
        fp_mem = _clip(float(getattr(app_risk, "trigger_false_positive_memory", 0.0)), 0.0, 1.0)
        P_fp = _clip(rc_pen + 0.55 * fp_mem, 0.0, 1.0)
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
            "D_corr": D_corr,
            "D_thesis": D_thesis,
            "D_liq": D_liq,
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

    meta: dict[str, Any] = {
        "schema_version": 1,
        "symbol": symbol,
        "thesis_bucket_by_direction": thesis_keys,
        "liquidation_cluster_key": liq_cluster_key,
        "long_short_structural_correlation_proxy": round(corr_ls, 6),
        "diversification_weights": {"d1": d1, "d2": d2, "d3": d3},
    }
    if sel_dir is not None and str(int(sel_dir)) in thesis_keys:
        meta["selected_thesis_bucket"] = thesis_keys[str(int(sel_dir))]

    result = AuctionResult(
        selected_symbol=symbol if winner is not None else None,
        selected_direction=sel_dir,
        selected_score=sel_score,
        records=records,
        top_n_limit=top_n,
        max_notional_usd=max_notional,
        selected_notional_usd=total_notional,
        clustering_metadata=meta,
    )
    return winner, result

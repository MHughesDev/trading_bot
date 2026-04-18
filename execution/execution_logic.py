"""APEX execution logic — confidence, style, stress, worst-case edge, partial-fill (FB-CAN-008)."""

from __future__ import annotations

from typing import Any

from app.contracts.execution_guidance import ExecutionFeedback, ExecutionGuidance
from app.contracts.orders import OrderIntent


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def _execution_domain(settings: Any) -> dict[str, Any]:
    """FB-CAN-047: thresholds from ``apex_canonical.domains.execution``."""
    if settings is None:
        return {}
    try:
        ex = settings.canonical.domains.execution
        return dict(ex) if isinstance(ex, dict) else {}
    except Exception:
        return {}


def _f(ctx: dict[str, Any], key: str, default: float) -> float:
    v = ctx.get(key)
    if v is None:
        return default
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


def compute_execution_confidence(ctx: dict[str, Any]) -> float:
    """Spec §4.3 — five quality scores averaged."""
    qd = _clip01(_f(ctx, "depth_quality", 0.75))
    qs = _clip01(_f(ctx, "spread_quality", 0.75))
    qv = _clip01(_f(ctx, "venue_quality", 0.8))
    ql = _clip01(_f(ctx, "latency_quality", 0.75))
    qr = _clip01(_f(ctx, "slippage_quality", 0.7))
    raw = (qd + qs + qv + ql + qr) / 5.0
    return _clip01(raw)


def compute_stress_mode(ctx: dict[str, Any]) -> tuple[bool, list[str]]:
    """Spec §7 — stress when spread, heat, vol, or venue collapse."""
    reasons: list[str] = []
    spread_bps = _f(ctx, "spread_bps", 5.0)
    heat = _f(ctx, "heat_score", 0.2)
    vol = _f(ctx, "volatility_proxy", 0.02)
    vq = _f(ctx, "venue_quality", 0.8)
    liq_frag = _f(ctx, "liquidation_fragility", 0.0)

    stress = False
    if spread_bps > 45:
        stress = True
        reasons.append("stress_spread_widening")
    if heat > 0.78:
        stress = True
        reasons.append("stress_heat_extreme")
    if vq < 0.35:
        stress = True
        reasons.append("stress_venue_degradation")
    if vol > 0.08:
        stress = True
        reasons.append("stress_volatility")
    if liq_frag > 0.72:
        stress = True
        reasons.append("stress_liquidity_collapse")
    return stress, reasons


def compute_worst_case_edge(ctx: dict[str, Any]) -> tuple[float, bool]:
    """Spec §6 — worst_case_edge vs minimum_tradeable_edge."""
    expected = _f(ctx, "expected_edge", 0.015)
    exp_slip = _f(ctx, "expected_slippage_bps", 8.0) / 10_000.0
    wc_mult = _f(ctx, "worst_case_slippage_multiplier", 2.5)
    adverse = _f(ctx, "adverse_fill_penalty", 0.002)
    spread_pen = _f(ctx, "spread_risk_penalty", 0.001)
    worst_slip = exp_slip * wc_mult
    wce = expected - worst_slip - adverse - spread_pen
    min_edge = _f(ctx, "minimum_tradeable_edge", 0.002)
    suppress = wce < min_edge
    return wce, suppress


def select_execution_style(
    *,
    exec_conf: float,
    spread_bps: float,
    urgency_high: bool,
    remaining_edge: float,
    stress: bool,
    domain: dict[str, Any] | None = None,
) -> tuple[str, list[str]]:
    """Spec §5.2 — deterministic style selection + rationale codes (FB-CAN-047)."""
    dom = domain or {}
    high_t = float(dom.get("high_confidence_threshold", 0.72))
    med_t = float(dom.get("medium_confidence_threshold", 0.45))
    passive_spread = float(dom.get("passive_spread_limit_bps", 18.0))
    emergency_floor = float(dom.get("emergency_remaining_edge_floor", 0.001))
    stress_ec = float(dom.get("stress_twap_exec_conf_below", 0.25))

    if stress and exec_conf < stress_ec:
        return "twap", ["style_branch_stress_low_exec_conf_twap"]
    if exec_conf >= high_t and spread_bps <= passive_spread and not stress:
        return "passive", ["style_branch_passive_high_conf_tight_spread"]
    if exec_conf >= med_t:
        return "staggered", ["style_branch_staggered_medium_conf"]
    if urgency_high and remaining_edge > emergency_floor:
        return "aggressive", ["style_branch_aggressive_urgency_remaining_edge"]
    return "twap", ["style_branch_default_twap"]


def build_execution_guidance(ctx: dict[str, Any]) -> ExecutionGuidance:
    """Full guidance record for metadata + service gating."""
    dom = ctx.get("_execution_policy") if isinstance(ctx.get("_execution_policy"), dict) else {}
    spread_bps = _f(ctx, "spread_bps", 5.0)
    exec_conf = compute_execution_confidence(ctx)
    stress, stress_reasons = compute_stress_mode(ctx)
    wce, edge_suppress = compute_worst_case_edge(ctx)
    heat = _f(ctx, "heat_score", 0.2)
    urgency = bool(ctx.get("urgency_high", False))
    remaining_edge = _f(ctx, "remaining_edge", wce + 0.01)
    ec_floor = float(dom.get("suppress_exec_conf_below", 0.12))

    reasons: list[str] = []
    reasons.extend(stress_reasons)

    style, style_codes = select_execution_style(
        exec_conf=exec_conf,
        spread_bps=spread_bps,
        urgency_high=urgency,
        remaining_edge=remaining_edge,
        stress=stress,
        domain=dom,
    )

    suppress = edge_suppress or (exec_conf < ec_floor and not urgency)
    if edge_suppress:
        reasons.append("worst_case_edge_below_min")
    if exec_conf < ec_floor and not urgency:
        reasons.append("execution_confidence_floor")

    size_mult = 1.0
    if stress:
        size_mult *= min(1.0, max(0.35, 1.0 - 0.45 * heat))
    if exec_conf < 0.35:
        size_mult *= min(1.0, max(0.4, exec_conf / 0.35))
    size_mult = max(0.0, min(1.0, size_mult))

    max_slip = 25.0 + spread_bps * 0.8
    if stress:
        max_slip *= 0.75

    if suppress:
        style = "suppress"
        style_codes = ["style_branch_suppress"]

    return ExecutionGuidance(
        preferred_execution_style=style,
        execution_confidence=exec_conf,
        max_slippage_tolerance_bps=max_slip,
        stress_mode_flag=stress,
        venue_preference_order=list(ctx.get("venue_preference_order") or []),
        execution_reason_codes=reasons,
        style_rationale_codes=style_codes,
        worst_case_edge=wce,
        remaining_edge=remaining_edge,
        urgency_high=urgency,
        suppress_order=suppress,
        size_multiplier=size_mult,
    )


def reconcile_partial_fill(
    *,
    intended_qty: float,
    fill_ratio: float,
    remaining_edge: float,
    min_remaining_fraction: float,
    minimum_tradeable_edge: float,
    execution_confidence_realized: float,
    low_execution_floor: float,
) -> str:
    """Spec §8.3 — discrete outcome for gateway/replay."""
    remaining_fraction = 1.0 - fill_ratio
    if remaining_fraction <= min_remaining_fraction:
        return "done"
    if remaining_edge < minimum_tradeable_edge:
        return "abandon"
    if execution_confidence_realized < low_execution_floor:
        return "pause_or_reduce"
    return "continue_staggered"


def apply_execution_feedback(
    symbol: str,
    feedback: ExecutionFeedback,
    *,
    state: dict[str, dict[str, float]] | None = None,
) -> dict[str, float]:
    """Spec §9.4 — slow trust update; returns updated snapshot for symbol."""
    bucket = dict(state.get(symbol, {})) if state is not None else {}
    trust = float(bucket.get("execution_trust", 0.75))
    vq = float(bucket.get("venue_quality", 0.8))

    slip_pen = min(0.15, max(0.0, feedback.realized_slippage_bps / 500.0))
    trust = max(0.05, min(1.0, trust - slip_pen * 0.4))
    vq = max(0.05, min(1.0, 0.85 * vq + 0.15 * feedback.venue_quality_score))

    if feedback.partial_fill_flag and feedback.fill_ratio < 0.65:
        trust = max(0.05, trust - 0.03)

    bucket["execution_trust"] = trust
    bucket["venue_quality"] = vq
    return bucket


def build_execution_context_from_decision(
    *,
    spread_bps: float,
    feature_row: dict[str, float],
    regime: Any,
    forecast: Any,
    risk: Any,
    mid_price: float,
    forecast_packet: Any | None = None,
    execution_feedback_bucket: dict[str, float] | None = None,
    settings: Any | None = None,
) -> dict[str, Any]:
    """Build `execution_context` for guidance from live/replay decision outputs."""
    close = max(float(feature_row.get("close", mid_price)), 1e-12)
    atr = float(feature_row.get("atr_14", 0.0) or 0.0)
    vol_proxy = min(0.25, atr / close * 25.0)
    spread_stress = _clip01(spread_bps / 80.0)
    spread_quality = _clip01(1.0 - spread_stress * 1.1)
    depth_quality = _clip01(1.0 - spread_stress * 0.9 + min(0.15, float(feature_row.get("volume", 0.0) or 0.0) / (close * 1e6)))
    heat = 0.35
    trig_c = 0.4
    trig_s = 0.4
    apex = getattr(regime, "apex", None)
    if apex is not None:
        heat = float(apex.heat_score)
    if forecast_packet is not None:
        td = forecast_packet.forecast_diagnostics or {}
        tr = td.get("trigger")
        if isinstance(tr, dict):
            trig_c = float(tr.get("trigger_confidence", trig_c))
            trig_s = float(tr.get("trigger_strength", trig_s))
    ret1 = abs(float(feature_row.get("return_1", 0.0) or 0.0))
    expected_edge = max(0.001, min(0.08, abs(float(getattr(forecast, "returns_1", 0.0))) + ret1 * 0.5))
    slip_bps = max(2.0, spread_bps * 0.85 + float(getattr(forecast, "volatility", 0.02)) * 400.0)
    venue_q = _clip01(0.82 - spread_stress * 0.35)
    latency_q = _clip01(0.88 - spread_stress * 0.25)
    slip_q = _clip01(1.0 - min(0.9, slip_bps / 120.0))
    if execution_feedback_bucket:
        venue_q = _clip01(0.5 * venue_q + 0.5 * float(execution_feedback_bucket.get("venue_quality", venue_q)))
        slip_q = _clip01(0.55 * slip_q + 0.45 * float(execution_feedback_bucket.get("execution_trust", slip_q)))

    ex_dom = _execution_domain(settings)
    min_trade_edge = float(ex_dom.get("minimum_tradeable_edge", 0.0015))
    urg_s = float(ex_dom.get("urgency_trigger_strength_above", 0.55))
    urg_c = float(ex_dom.get("urgency_trigger_confidence_above", 0.4))
    rem_scale = float(ex_dom.get("remaining_edge_scale", 0.9))

    ctx: dict[str, Any] = {
        "spread_bps": spread_bps,
        "depth_quality": depth_quality,
        "spread_quality": spread_quality,
        "venue_quality": venue_q,
        "latency_quality": latency_q,
        "slippage_quality": slip_q,
        "heat_score": heat,
        "volatility_proxy": vol_proxy,
        "expected_edge": expected_edge,
        "expected_slippage_bps": slip_bps,
        "worst_case_slippage_multiplier": float(ex_dom.get("worst_case_slippage_multiplier", 2.5)),
        "adverse_fill_penalty": 0.001 + spread_stress * 0.002,
        "spread_risk_penalty": spread_stress * 0.004,
        "minimum_tradeable_edge": min_trade_edge,
        "remaining_edge": max(0.0, expected_edge * rem_scale - min_trade_edge * 0.25),
        "urgency_high": trig_s > urg_s and trig_c > urg_c,
        "liquidation_fragility": min(1.0, heat * 0.6 + vol_proxy * 2.0),
    }
    return ctx


def extract_execution_context(intent: OrderIntent) -> dict[str, Any]:
    """Merge `metadata.execution_context` with top-level metadata hints."""
    meta = intent.metadata or {}
    ctx = dict(meta.get("execution_context") or {})
    if "spread_bps" not in ctx and "spread_bps" in meta:
        ctx["spread_bps"] = meta["spread_bps"]
    return ctx


def merge_guidance_into_intent(intent: OrderIntent, guidance: ExecutionGuidance) -> OrderIntent:
    """Attach serialized guidance for adapters/logs."""
    meta = dict(intent.metadata or {})
    meta["execution_guidance"] = guidance.model_dump()
    return intent.model_copy(update={"metadata": meta})


def ensure_risk_signature(intent: OrderIntent, settings: Any) -> OrderIntent:
    """Sign intent when secret configured and not already signed."""
    from app.config.settings import AppSettings

    if not isinstance(settings, AppSettings):
        return intent
    if intent.metadata.get("risk_signature"):
        return intent
    secret = settings.risk_signing_secret.get_secret_value() if settings.risk_signing_secret else None
    if not secret:
        return intent
    from risk_engine.signing import sign_order_intent

    return sign_order_intent(intent, secret)


def prepare_order_intent_for_execution(
    intent: OrderIntent,
    settings: Any,
    execution_context: dict[str, Any] | None = None,
) -> OrderIntent | None:
    """Apply APEX execution guidance, optional size scale, then risk-sign.

    Pass ``sign=False`` from :meth:`RiskEngine.to_order_intent` so guidance is included
    in the signed payload. Already-signed intents are passed through unchanged.
    """
    meta = intent.metadata or {}
    if meta.get("risk_signature"):
        return intent
    if meta.get("skip_execution_guidance") or meta.get("flatten_stop"):
        out = intent.model_copy(deep=True)
        out.metadata = dict(out.metadata or {})
        out.metadata["execution_prepared"] = True
        return ensure_risk_signature(out, settings)

    ctx = dict(execution_context or extract_execution_context(intent))
    if "_execution_policy" not in ctx and settings is not None:
        ctx["_execution_policy"] = _execution_domain(settings)
    guidance = build_execution_guidance(ctx)
    out = merge_guidance_into_intent(intent, guidance)
    if guidance.suppress_order:
        return None
    if guidance.size_multiplier < 1.0 - 1e-12:
        from decimal import Decimal

        q = intent.quantity * Decimal(str(guidance.size_multiplier))
        out = out.model_copy(update={"quantity": q})
    out.metadata = dict(out.metadata or {})
    out.metadata["execution_prepared"] = True
    return ensure_risk_signature(out, settings)

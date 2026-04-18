"""Build canonical DecisionRecord + related outputs from one tick (FB-CAN-036)."""

from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime
from typing import Any

from app.config.settings import AppSettings
from app.contracts.decision_record import (
    DecisionOutcome,
    DecisionRecord,
    NoTradeDecision,
    PreferredExecutionStyle,
    ReduceExposureIntent,
    SafetyOverrideEvent,
    SafetyOverrideType,
    SuppressionEvent,
    SuppressionType,
    TradeIntentCanonical,
    TradeIntentSide,
    Urgency,
)
from app.contracts.reason_codes import (
    PIP_CARRY_SLEEVE_BLOCKED,
    PIP_TRADE_SELECTED,
    normalize_reason_codes,
)
from app.contracts.decisions import ActionProposal, RouteDecision, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState, SystemMode
from execution.execution_logic import (
    _execution_domain,
    build_execution_context_from_decision,
    build_execution_guidance,
)


def _ts(ts: datetime | None) -> datetime:
    t = ts if ts is not None else datetime.now(UTC)
    if t.tzinfo is None:
        t = t.replace(tzinfo=UTC)
    return t


def _cfg_versions(settings: AppSettings) -> tuple[str, str | None]:
    try:
        cv = str(settings.canonical.metadata.config_version)
        lv = settings.canonical.metadata.logic_version
        return cv, lv
    except Exception:
        return "1.0.0", None


def _deterministic_id(prefix: str, symbol: str, ts: datetime, extra: str = "") -> str:
    t = ts.replace(microsecond=0)
    raw = f"{prefix}|{symbol}|{t.isoformat()}|{extra}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:24]


def build_decision_record(
    *,
    symbol: str,
    data_timestamp: datetime | None,
    settings: AppSettings,
    regime: RegimeOutput,
    forecast: ForecastOutput,
    route: RouteDecision,
    proposal: ActionProposal | None,
    risk: RiskState,
    forecast_packet: ForecastPacket | None,
    trade: TradeAction | None,
    feature_row: dict[str, float] | None = None,
    mid_price: float | None = None,
) -> DecisionRecord:
    """Assemble §15 decision record from pipeline + risk outputs."""
    ts = _ts(data_timestamp)
    rid = _deterministic_id(
        "dr",
        symbol,
        ts,
        json.dumps(forecast.model_dump(mode="json"), sort_keys=True),
    )
    cv, lv = _cfg_versions(settings)

    fd: dict[str, Any] = {}
    trig: dict[str, Any] | None = None
    auct: dict[str, Any] | None = None
    boundary_ids: dict[str, str] = {}
    if forecast_packet is not None:
        fd = forecast_packet.forecast_diagnostics or {}
        trig = fd.get("trigger") if isinstance(fd.get("trigger"), dict) else None
        raw_a = fd.get("auction")
        if isinstance(raw_a, dict):
            auct = raw_a
        elif hasattr(raw_a, "model_dump"):
            auct = raw_a.model_dump(mode="json")
        cbi = fd.get("canonical_boundary_input")
        if isinstance(cbi, dict):
            for k in ("market", "structural", "safety", "execution_feedback", "service_config"):
                sub = cbi.get(k)
                if isinstance(sub, dict) and "snapshot_id" in sub:
                    sid_val = str(sub["snapshot_id"])
                    boundary_ids[f"{k}_snapshot_id"] = sid_val
                    if k == "service_config":
                        boundary_ids["service_config_snapshot_id"] = sid_val

    apex = getattr(regime, "apex", None)
    deg = getattr(apex, "degradation", None) if apex is not None else None
    deg_s = deg.value if deg is not None and hasattr(deg, "value") else str(deg or "")

    eff: dict[str, float] = {}

    fc_sum = forecast.model_dump(mode="json")
    fc_sum["route_confidence"] = float(route.confidence)
    if forecast_packet is not None:
        fc_sum["packet_ood"] = forecast_packet.ood_score

    pipe_codes = normalize_reason_codes(list(risk.last_pipeline_no_trade_codes or []))
    risk_codes = normalize_reason_codes(list(risk.last_risk_block_codes or []))
    ho = fd.get("hard_override") if isinstance(fd.get("hard_override"), dict) else {}
    ho_active = bool(ho.get("active")) if ho else bool(getattr(risk, "hard_override_active", False))

    safety_evt: SafetyOverrideEvent | None = None
    if ho_active:
        kind = str(ho.get("kind") or getattr(risk, "hard_override_kind", "none"))
        safety_evt = SafetyOverrideEvent(
            event_id=f"so-{rid[:8]}",
            timestamp=ts,
            override_type=SafetyOverrideType.HARD_OVERRIDE,
            reason_codes=normalize_reason_codes([f"hard_override_{kind}"]),
            affected_instruments=[symbol],
        )

    outcome = DecisionOutcome.NO_TRADE
    trade_intent: TradeIntentCanonical | None = None
    reduce_intent: ReduceExposureIntent | None = None
    no_trade: NoTradeDecision | None = None
    suppression: SuppressionEvent | None = None
    guid: Any | None = None

    if risk.mode == SystemMode.FLATTEN_ALL and trade is not None:
        outcome = DecisionOutcome.REDUCE_EXPOSURE
        reduce_intent = ReduceExposureIntent(
            intent_id=f"ri-{rid[:8]}",
            timestamp=ts,
            instrument_id=symbol,
            reason_codes=normalize_reason_codes(["flatten_all"]),
        )
    elif trade is not None and proposal is not None:
        outcome = DecisionOutcome.TRADE_INTENT
        side = TradeIntentSide.LONG if proposal.direction > 0 else TradeIntentSide.SHORT
        tc = float(risk.risk_trigger_confidence or 0.0)
        dc = float(route.confidence)
        fr = feature_row or {}
        mp = float(mid_price if mid_price is not None else fr.get("close", 1.0))
        sb = float(risk.spread_bps or 0.0)
        xctx = build_execution_context_from_decision(
            spread_bps=sb,
            feature_row=fr,
            regime=regime,
            forecast=forecast,
            risk=risk,
            mid_price=mp,
            forecast_packet=forecast_packet,
            execution_feedback_bucket=None,
            settings=settings,
        )
        xctx["_execution_policy"] = _execution_domain(settings)
        guid = build_execution_guidance(xctx)
        ec = float(guid.execution_confidence)
        style_map = {
            "passive": PreferredExecutionStyle.PASSIVE,
            "aggressive": PreferredExecutionStyle.AGGRESSIVE,
            "staggered": PreferredExecutionStyle.STAGGERED,
            "twap": PreferredExecutionStyle.TWAP,
            "suppress": PreferredExecutionStyle.SUPPRESS,
        }
        pstyle = style_map.get(
            str(guid.preferred_execution_style), PreferredExecutionStyle.STAGGERED
        )
        urg = Urgency.HIGH if guid.urgency_high else Urgency.MEDIUM
        trade_intent = TradeIntentCanonical(
            intent_id=f"ti-{rid[:8]}",
            timestamp=ts,
            instrument_id=symbol,
            side=side,
            urgency=urg,
            size_fraction=float(proposal.size_fraction),
            preferred_execution_style=pstyle,
            decision_confidence=dc,
            trigger_confidence=tc,
            execution_confidence=ec,
            degradation_level=deg_s or "normal",
            max_slippage_tolerance_bps=float(guid.max_slippage_tolerance_bps),
            reason_codes=normalize_reason_codes(
                [PIP_TRADE_SELECTED] + list(guid.style_rationale_codes)
            ),
        )
    else:
        codes: list[str] = []
        codes.extend(pipe_codes)
        codes.extend(risk_codes)
        if apex is not None:
            codes.extend(normalize_reason_codes(list(getattr(apex, "novelty_reason_codes", None) or [])))
            codes.extend(normalize_reason_codes(list(getattr(apex, "safety_reason_codes", None) or [])))
        if not codes:
            codes = normalize_reason_codes(["no_trade_unknown"])
        else:
            codes = normalize_reason_codes(codes)
        no_trade = NoTradeDecision(
            event_id=f"nt-{rid[:8]}",
            timestamp=ts,
            instrument_id=symbol,
            no_trade_reason_codes=codes,
            state_summary={
                "route": route.route_id.value,
                "regime": regime.semantic.value,
                "degradation": deg_s,
            },
        )
        if PIP_CARRY_SLEEVE_BLOCKED in pipe_codes:
            suppression = SuppressionEvent(
                event_id=f"su-{rid[:8]}",
                timestamp=ts,
                instrument_id=symbol,
                suppression_type=SuppressionType.CARRY_ISOLATION,
                reason_codes=pipe_codes,
                blocked_candidate_id="carry_directional",
                degradation_level=deg_s or "normal",
            )

    diag: dict[str, Any] = {
        "forecast_diagnostics_keys": sorted(fd.keys()) if fd else [],
        "auction_clustering": (auct or {}).get("clustering_metadata")
        if isinstance(auct, dict)
        else None,
    }
    if guid is not None:
        diag["execution_guidance_preview"] = guid.model_dump(mode="json")

    return DecisionRecord(
        record_id=rid,
        timestamp=ts,
        instrument_id=symbol,
        config_version=cv,
        logic_version=lv,
        input_snapshot_ids=boundary_ids,
        effective_signal_map=eff,
        regime_semantic=regime.semantic.value,
        degradation_level=deg_s or None,
        forecast_summary=fc_sum,
        trigger_output=trig,
        auction_summary=auct if isinstance(auct, dict) else None,
        selected_route=route.route_id.value,
        outcome=outcome,
        trade_intent=trade_intent,
        reduce_exposure_intent=reduce_intent,
        no_trade=no_trade,
        suppression=suppression,
        safety_override=safety_evt,
        risk_block_codes=risk_codes,
        pipeline_no_trade_codes=pipe_codes,
        diagnostics=diag,
    )


_LAST_DECISION_RECORD: dict[str, Any] | None = None


def set_last_decision_record(record: DecisionRecord) -> None:
    """Expose last tick for control plane (single-process operator view)."""
    global _LAST_DECISION_RECORD
    _LAST_DECISION_RECORD = record.model_dump(mode="json")


def get_last_decision_record() -> dict[str, Any] | None:
    return _LAST_DECISION_RECORD


__all__ = ["build_decision_record", "get_last_decision_record", "set_last_decision_record"]

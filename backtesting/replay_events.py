"""Build canonical replay event payloads from pipeline + risk state (FB-CAN-009)."""

from __future__ import annotations

from typing import Any

from app.contracts.replay_events import ReplayEventEnvelope


def build_market_snapshot_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    mid_price: float,
    spread_bps: float,
    feature_row: dict[str, float],
) -> ReplayEventEnvelope:
    return ReplayEventEnvelope(
        event_family="market_snapshot_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload={
            "mid_price": mid_price,
            "spread_bps": spread_bps,
            "close": feature_row.get("close"),
            "volume": feature_row.get("volume"),
        },
    )


def build_structural_signal_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    feature_row: dict[str, float],
) -> ReplayEventEnvelope:
    keys = sorted(feature_row.keys())
    head = [(k, round(float(feature_row[k]), 6)) for k in keys[:32]]
    return ReplayEventEnvelope(
        event_family="structural_signal_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload={
            "feature_dim": len(feature_row),
            "feature_keys_head": keys[:16],
            "feature_fingerprint": repr(head),
        },
    )


def build_safety_snapshot_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    regime: Any,
    risk: Any,
) -> ReplayEventEnvelope:
    apex = getattr(regime, "apex", None)
    payload: dict[str, Any] = {
        "regime_semantic": getattr(getattr(regime, "semantic", None), "value", None),
        "canonical_degradation": getattr(risk, "canonical_degradation", None),
        "canonical_size_multiplier": getattr(risk, "canonical_size_multiplier", None),
    }
    if apex is not None:
        payload["apex_degradation"] = getattr(apex, "degradation", None)
        payload["apex_heat"] = getattr(apex, "heat_score", None)
    return ReplayEventEnvelope(
        event_family="safety_snapshot_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload=payload,
    )


def build_decision_output_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    config_version: str,
    logic_version: str,
    regime: Any,
    forecast: Any,
    route: Any,
    proposal: Any,
    risk: Any,
    forecast_packet: Any | None,
) -> ReplayEventEnvelope:
    trig = None
    auct = None
    if forecast_packet is not None:
        fd = forecast_packet.forecast_diagnostics or {}
        trig = fd.get("trigger")
        auct = fd.get("auction")
    rs = getattr(risk, "last_risk_sizing", None)
    return ReplayEventEnvelope(
        event_family="decision_output_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload={
            "config_version": config_version,
            "logic_version": logic_version,
            "regime": regime.model_dump(mode="json") if hasattr(regime, "model_dump") else {},
            "forecast": forecast.model_dump(mode="json") if hasattr(forecast, "model_dump") else {},
            "route": route.model_dump(mode="json") if hasattr(route, "model_dump") else {},
            "proposal": proposal.model_dump(mode="json") if proposal is not None else None,
            "risk_last_risk_sizing": rs,
            "trigger": trig,
            "auction": auct,
        },
    )


def build_execution_feedback_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    simulated_fill_price: float | None,
    simulated_fill_ratio: float,
    simulated_latency_ms: float,
    execution_confidence_realized: float,
    profile: str,
) -> ReplayEventEnvelope:
    return ReplayEventEnvelope(
        event_family="execution_feedback_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload={
            "simulated_fill_price": simulated_fill_price,
            "simulated_fill_ratio": simulated_fill_ratio,
            "simulated_fill_latency_ms": simulated_latency_ms,
            "execution_confidence_realized": execution_confidence_realized,
            "execution_model_profile": profile,
        },
    )


def build_fault_injection_event(
    *,
    replay_run_id: str,
    symbol: str,
    timestamp: Any,
    reasons: list[str],
    profile: dict[str, Any],
) -> ReplayEventEnvelope:
    return ReplayEventEnvelope(
        event_family="fault_injection_event",
        replay_run_id=replay_run_id,
        symbol=symbol,
        timestamp=timestamp,
        payload={"reason_codes": reasons, "profile": profile},
    )

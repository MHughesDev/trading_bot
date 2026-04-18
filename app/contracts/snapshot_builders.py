"""Build typed APEX snapshots from legacy feature rows + runtime context (FB-CAN-015)."""

from __future__ import annotations

import uuid
from datetime import UTC, datetime
from typing import Any

from app.config.settings import AppSettings
from app.config.signal_confidence import apply_signal_family_confidence
from app.contracts.decision_snapshots import (
    DecisionBoundaryInput,
    ExchangeRiskLevel,
    ExecutionFeedbackSnapshot,
    MarketSnapshot,
    OrderStyleUsed,
    SafetyRegimeSnapshot,
    ServiceConfigurationSnapshot,
    SessionMode,
    StructuralSignalSnapshot,
)


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


def build_decision_boundary_input(
    *,
    symbol: str,
    feature_row: dict[str, float],
    spread_bps: float,
    mid_price: float,
    data_timestamp: datetime | None,
    settings: AppSettings,
    execution_feedback: ExecutionFeedbackSnapshot | None = None,
) -> tuple[DecisionBoundaryInput, dict[str, float]]:
    """
    Validate/normalize inputs into :class:`DecisionBoundaryInput` and merge canonical
    scalar hints into a **copy** of ``feature_row`` for downstream engines that still
    read a flat dict.
    """
    ts = data_timestamp if data_timestamp is not None else datetime.now(UTC)
    if ts.tzinfo is None:
        ts = ts.replace(tzinfo=UTC)
    sid = f"{symbol}-{ts.replace(microsecond=0).isoformat()}"

    close = max(_safe_float(feature_row, "close", mid_price), 1e-12)
    vol = max(0.0, _safe_float(feature_row, "volume", 0.0))
    rsi = _safe_float(feature_row, "rsi_14", 50.0)
    atr = max(0.0, _safe_float(feature_row, "atr_14", 0.0))

    sp = max(0.0, float(spread_bps))
    half = float(mid_price) * (sp / 20_000.0)
    bid = float(mid_price) - half
    ask = float(mid_price) + half

    rv_short = _clip01(atr / close * 30.0)
    rv_med = _clip01(atr / close * 25.0)
    imb = _clip01((rsi - 50.0) / 50.0)
    burst = _clip01(vol / (close * 1e-6 + 1e6))

    merged: dict[str, float] = dict(feature_row)
    try:
        dom = settings.canonical.domains
        merged = apply_signal_family_confidence(
            merged,
            signal_confidence=dict(dom.signal_confidence or {}),
            feature_families=dict(dom.feature_families or {}),
        )
    except Exception:
        pass

    m_fresh = _clip01(_safe_float(merged, "feature_freshness", 0.92))
    m_rel = _clip01(_safe_float(merged, "feature_reliability", 0.88))

    market = MarketSnapshot(
        snapshot_id=sid,
        timestamp=ts,
        instrument_id=symbol,
        venue_group="kraken",
        last_price=close,
        mid_price=float(mid_price),
        best_bid=bid,
        best_ask=ask,
        spread_bps=sp,
        realized_vol_short=rv_short,
        realized_vol_medium=rv_med,
        book_imbalance=imb,
        depth_near_touch=1.0,
        trade_volume_short=vol,
        volume_burst_score=burst,
        market_freshness=m_fresh,
        market_reliability=m_rel,
        session_mode=SessionMode.WEEKEND if ts.weekday() >= 5 else SessionMode.REGULAR,
        price_return_short=_safe_float(merged, "return_1", 0.0),
    )

    structural = StructuralSignalSnapshot(
        snapshot_id=sid,
        timestamp=ts,
        instrument_id=symbol,
        funding_rate=_safe_float(merged, "funding_rate", 0.0),
        funding_rate_zscore=_safe_float(merged, "funding_rate_zscore", 0.0),
        funding_velocity=_safe_float(merged, "funding_velocity", 0.0),
        open_interest=_safe_float(merged, "open_interest", 0.0),
        open_interest_delta_short=_safe_float(merged, "open_interest_delta_short", 0.0),
        basis_bps=_safe_float(merged, "basis_bps", 0.0),
        cross_exchange_divergence=_safe_float(merged, "cross_exchange_divergence", 0.0),
        liquidation_proximity_long=_clip01(_safe_float(merged, "liquidation_proximity_long", 0.5)),
        liquidation_proximity_short=_clip01(_safe_float(merged, "liquidation_proximity_short", 0.5)),
        liquidation_cluster_density_long=_clip01(_safe_float(merged, "liquidation_cluster_density_long", 0.0)),
        liquidation_cluster_density_short=_clip01(_safe_float(merged, "liquidation_cluster_density_short", 0.0)),
        liquidation_data_confidence=_clip01(_safe_float(merged, "liquidation_data_confidence", 0.5)),
        perp_spot_divergence_score=_safe_float(merged, "perp_spot_divergence_score", 0.0)
        if "perp_spot_divergence_score" in merged
        else None,
        gex_score=_safe_float(merged, "gex_score", 0.0) if "gex_score" in merged else None,
        iv_skew_score=_safe_float(merged, "iv_skew_score", 0.0) if "iv_skew_score" in merged else None,
        options_freshness=_clip01(_safe_float(merged, "options_freshness", 0.0))
        if float(merged.get("options_context_available", 0.0) or 0.0) >= 0.5
        else None,
        options_reliability=_clip01(_safe_float(merged, "options_reliability", 0.0))
        if float(merged.get("options_context_available", 0.0) or 0.0) >= 0.5
        else None,
        stablecoin_flow_proxy=_safe_float(merged, "stablecoin_flow_proxy", 0.0)
        if "stablecoin_flow_proxy" in merged
        else None,
        stablecoin_freshness=_clip01(_safe_float(merged, "stablecoin_freshness", 0.0))
        if float(merged.get("stablecoin_flow_available", 0.0) or 0.0) >= 0.5
        else None,
        signal_freshness_structural=_clip01(_safe_float(merged, "structural_freshness", 0.75)),
        signal_reliability_structural=_clip01(_safe_float(merged, "structural_reliability", 0.7)),
        signal_source_count=min(
            7,
            int(round(5.0 * _clip01(_safe_float(merged, "structural_family_coverage", 0.0))))
            + (1 if _safe_float(merged, "options_context_available", 0.0) >= 0.5 else 0)
            + (1 if _safe_float(merged, "stablecoin_flow_available", 0.0) >= 0.5 else 0),
        ),
    )

    safety = SafetyRegimeSnapshot(
        snapshot_id=sid,
        timestamp=ts,
        instrument_id=symbol,
        regime_probabilities={
            "trend": 0.2,
            "range": 0.2,
            "stress": 0.2,
            "dislocated": 0.2,
            "transition": 0.2,
        },
        regime_confidence=0.2,
        transition_probability=0.3,
        novelty_score=0.25,
        crypto_heat_score=0.3,
        reflexivity_score=0.3,
        degradation_level="normal",
        weekend_mode=ts.weekday() >= 5,
        exchange_risk_level=ExchangeRiskLevel.LOW,
    )

    if execution_feedback is None:
        execution_feedback = ExecutionFeedbackSnapshot(
            feedback_id=f"fb-{uuid.uuid4().hex[:12]}",
            timestamp=ts,
            instrument_id=symbol,
            expected_fill_price=float(mid_price),
            realized_fill_price=float(mid_price),
            realized_slippage_bps=0.0,
            fill_ratio=1.0,
            fill_latency_ms=0.0,
            execution_confidence_realized=0.75,
            venue_quality_score=0.85,
            partial_fill_flag=False,
            order_style_used=OrderStyleUsed.MARKET,
        )

    cv = "1.0.0"
    lv = None
    try:
        cv = str(settings.canonical.metadata.config_version)
        lv = settings.canonical.metadata.logic_version
    except Exception:
        pass

    svc = ServiceConfigurationSnapshot(
        snapshot_id=sid,
        timestamp=ts,
        config_version=cv,
        logic_version=lv,
        execution_mode=str(settings.execution_mode),
        market_data_symbols=list(settings.market_data_symbols),
        bar_interval_seconds=int(settings.market_data_bar_interval_seconds),
    )

    bundle = DecisionBoundaryInput(
        market=market,
        structural=structural,
        safety=safety,
        execution_feedback=execution_feedback,
        service_config=svc,
    )

    merged["canonical_market_freshness"] = float(m_fresh)
    merged["canonical_market_reliability"] = m_rel
    merged["canonical_structural_freshness"] = structural.signal_freshness_structural
    merged["canonical_structural_reliability"] = float(structural.signal_reliability_structural)
    merged["canonical_safety_heat"] = float(safety.crypto_heat_score)
    merged["canonical_safety_novelty"] = float(safety.novelty_score)
    merged["canonical_exec_slippage_bps"] = float(execution_feedback.realized_slippage_bps)
    merged["canonical_exec_fill_ratio"] = float(execution_feedback.fill_ratio)
    merged["canonical_book_imbalance"] = float(imb)

    return bundle, merged


def boundary_input_to_diagnostic_dict(inp: DecisionBoundaryInput) -> dict[str, Any]:
    """JSON-serializable summary for ForecastPacket.forecast_diagnostics."""
    return {
        "market": inp.market.model_dump(mode="json"),
        "structural": inp.structural.model_dump(mode="json"),
        "safety": inp.safety.model_dump(mode="json"),
        "execution_feedback": inp.execution_feedback.model_dump(mode="json"),
        "service_config": inp.service_config.model_dump(mode="json"),
    }

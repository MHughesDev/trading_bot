"""APEX canonical monitoring metric families (FB-CAN-010, FB-CAN-039).

Maps domains in docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md
to Prometheus instruments. Call :func:`record_canonical_post_tick` once per decision cycle.
"""

from __future__ import annotations

from typing import Any

from prometheus_client import Counter, Gauge, Histogram

from observability.drift_calibration_metrics import record_calibration_and_drift_from_tick

# --- State / safety (spec §4.3) ---
CANONICAL_REGIME_CONFIDENCE = Histogram(
    "tb_canonical_regime_confidence",
    "APEX regime confidence (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_HEAT_SCORE = Histogram(
    "tb_canonical_heat_score",
    "APEX crypto heat score (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_NOVELTY = Histogram(
    "tb_canonical_novelty",
    "APEX novelty score (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_REFLEXIVITY = Histogram(
    "tb_canonical_reflexivity_score",
    "APEX reflexivity score (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_HEAT_COMPONENT = Histogram(
    "tb_canonical_heat_component",
    "APEX heat score raw components Hf..He before weighted sum (spec §8.2)",
    ["symbol", "component"],
    buckets=(0.0, 0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0),
)
CANONICAL_NOVELTY_REASON = Counter(
    "tb_canonical_novelty_reason_total",
    "Novelty reason codes emitted with state (FB-CAN-042)",
    ["symbol", "reason"],
)
CANONICAL_DEGRADATION_TICKS = Counter(
    "tb_canonical_degradation_observations_total",
    "Per-tick degradation level observations (rate ~ occupancy)",
    ["symbol", "level"],
)
CANONICAL_HARD_OVERRIDE = Counter(
    "tb_canonical_hard_override_total",
    "Hard override classified this tick (APEX safety taxonomy)",
    ["symbol", "kind"],
)
CANONICAL_DEGRADATION_TRANSITION_COUNT = Gauge(
    "tb_canonical_degradation_transition_count",
    "Cumulative degradation level transitions observed on RiskState",
    ["symbol"],
)

# --- Trigger (spec §4.4) ---
CANONICAL_TRIGGER_STAGE = Counter(
    "tb_canonical_trigger_stage_total",
    "Trigger stage outcomes per decision tick",
    ["symbol", "stage", "passed"],
)
CANONICAL_TRIGGER_MISSED_MOVE = Counter(
    "tb_canonical_trigger_missed_move_total",
    "Missed-move flag set on trigger evaluation",
    ["symbol"],
)
CANONICAL_TRIGGER_SETUP_TO_CONFIRM_LATENCY_MS = Histogram(
    "tb_canonical_trigger_setup_to_confirm_latency_ms",
    "Deterministic setup→confirm stage ordering latency (FB-CAN-043)",
    ["symbol"],
    buckets=(0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0, 25.0, 50.0, 100.0),
)

# --- Auction / decision (spec §4.5) ---
CANONICAL_AUCTION_SCORE = Histogram(
    "tb_canonical_auction_selected_score",
    "Auction selected score when present",
    ["symbol"],
    buckets=(-1.0, -0.5, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_AUCTION_SUPPRESSED = Counter(
    "tb_canonical_auction_suppressed_total",
    "Cycles where auction had no winner (suppress or outranked)",
    ["symbol"],
)
CANONICAL_AUCTION_TOP_N_SATURATION = Histogram(
    "tb_canonical_auction_top_n_saturation",
    "Auction top-N saturation selected_count/top_n (FB-CAN-044)",
    ["symbol"],
    buckets=(0.0, 0.25, 0.5, 0.75, 1.0),
)
CANONICAL_AUCTION_CANDIDATES_EVALUATED = Histogram(
    "tb_canonical_auction_candidates_evaluated",
    "Candidates scored in opportunity auction this tick",
    ["symbol"],
    buckets=(0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 8.0),
)

# --- Risk (spec §4.6) ---
CANONICAL_SIZE_MULTIPLIER = Histogram(
    "tb_canonical_risk_size_multiplier",
    "Canonical size multiplier applied in risk engine",
    ["symbol"],
    buckets=(0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0),
)
CANONICAL_FINAL_NOTIONAL_USD = Histogram(
    "tb_canonical_risk_final_notional_usd",
    "Final notional USD after layered sizing (when recorded)",
    ["symbol"],
    buckets=(0.0, 100.0, 500.0, 1000.0, 5000.0, 10000.0, 25000.0, 50000.0, 100000.0),
)

# --- Execution guidance (FB-CAN-047) ---
CANONICAL_EXECUTION_STYLE = Counter(
    "tb_canonical_execution_style_total",
    "Selected preferred_execution_style from pre-trade guidance preview",
    ["symbol", "style"],
)
CANONICAL_EXECUTION_GUIDANCE_CONFIDENCE = Histogram(
    "tb_canonical_execution_guidance_confidence",
    "execution_confidence from ExecutionGuidance preview on trade_intent ticks",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)

# --- Optional feature families (FB-CAN-050) ---
CANONICAL_OPTIONS_CONTEXT_AVAILABLE = Gauge(
    "tb_canonical_options_context_available",
    "1 when gex_score or iv_skew_score present on feature row",
    ["symbol"],
)
CANONICAL_STABLECOIN_FLOW_AVAILABLE = Gauge(
    "tb_canonical_stablecoin_flow_available",
    "1 when stablecoin_flow_proxy present on feature row",
    ["symbol"],
)
CANONICAL_OPTIONS_CONTEXT_FALLBACK = Gauge(
    "tb_canonical_options_context_fallback_active",
    "1 when options family enabled but no options fields upstream",
    ["symbol"],
)
CANONICAL_STABLECOIN_FLOW_FALLBACK = Gauge(
    "tb_canonical_stablecoin_flow_fallback_active",
    "1 when stablecoin family enabled but no stablecoin proxy upstream",
    ["symbol"],
)

# --- Data quality proxy (spec §4.2) ---
CANONICAL_DATA_AGE_SECONDS = Histogram(
    "tb_canonical_data_age_seconds",
    "RiskState data_age_seconds when set",
    ["symbol"],
    buckets=(0.0, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0),
)

# --- Drift / forecaster (spec §4.9) ---
CANONICAL_OOD_SCORE = Histogram(
    "tb_canonical_forecast_ood_score",
    "ForecastPacket OOD score",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)

# --- Governance (spec §4.10) — set from runtime if desired ---
CANONICAL_ACTIVE_CONFIG_VERSION = Gauge(
    "tb_canonical_active_config_version_info",
    "Active canonical config version (label value_info)",
    ["version"],
)

# --- Carry sleeve (spec §4.8) ---
CANONICAL_CARRY_SLEEVE_ACTIVE = Gauge(
    "tb_canonical_carry_sleeve_active",
    "Carry sleeve active this tick (0/1)",
    ["symbol"],
)
CANONICAL_CARRY_TARGET_NOTIONAL_USD = Histogram(
    "tb_canonical_carry_target_notional_usd",
    "Carry sleeve target notional when evaluated",
    ["symbol"],
    buckets=(0.0, 100.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 25000.0),
)
CANONICAL_CARRY_FUNDING_SIGNAL = Histogram(
    "tb_canonical_carry_funding_signal",
    "Funding extremity proxy used for carry (0-1)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_CARRY_TRIGGER_CONFIDENCE = Histogram(
    "tb_canonical_carry_trigger_confidence",
    "Trigger confidence at carry evaluation (0-1, FB-CAN-064)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_CARRY_DECISION_QUALITY = Histogram(
    "tb_canonical_carry_decision_quality",
    "Carry decision quality proxy funding×trigger_confidence (0-1, FB-CAN-064)",
    ["symbol"],
    buckets=(0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0),
)
CANONICAL_CARRY_REASON = Counter(
    "tb_canonical_carry_reason_total",
    "Carry sleeve reason codes emitted per tick (FB-CAN-064)",
    ["symbol", "reason"],
)
CANONICAL_CARRY_DIRECTIONAL_SUPPRESSION = Counter(
    "tb_canonical_carry_directional_suppression_total",
    "Ticks where carry isolation suppressed directional flow (FB-CAN-064)",
    ["symbol"],
)
CANONICAL_SHADOW_DIVERGENCE = Counter(
    "tb_canonical_replay_shadow_divergence_total",
    "Shadow vs live divergence events (increment when comparison runs)",
    ["kind"],
)


def record_canonical_post_tick(
    *,
    symbol: str,
    regime: Any,
    risk: Any,
    forecast_packet: Any | None,
    carry_sleeve: dict[str, Any] | None = None,
    feature_row: dict[str, float] | None = None,
) -> None:
    """Record canonical metrics from one `run_decision_tick` completion."""
    sym = symbol or "unknown"

    apex = getattr(regime, "apex", None)
    if apex is not None:
        CANONICAL_REGIME_CONFIDENCE.labels(symbol=sym).observe(float(apex.regime_confidence))
        CANONICAL_HEAT_SCORE.labels(symbol=sym).observe(float(apex.heat_score))
        CANONICAL_NOVELTY.labels(symbol=sym).observe(float(apex.novelty))
        CANONICAL_REFLEXIVITY.labels(symbol=sym).observe(float(apex.reflexivity_score))
        hc = getattr(apex, "heat_components", None) or {}
        if isinstance(hc, dict):
            for comp_name, val in hc.items():
                try:
                    CANONICAL_HEAT_COMPONENT.labels(symbol=sym, component=str(comp_name)).observe(
                        float(val)
                    )
                except (TypeError, ValueError):
                    pass
        for code in getattr(apex, "novelty_reason_codes", None) or []:
            CANONICAL_NOVELTY_REASON.labels(symbol=sym, reason=str(code)).inc()
        deg = getattr(apex.degradation, "value", str(apex.degradation))
        CANONICAL_DEGRADATION_TICKS.labels(symbol=sym, level=str(deg)).inc()

    if risk is not None and bool(getattr(risk, "hard_override_active", False)):
        hk = getattr(risk, "hard_override_kind", None)
        k = getattr(hk, "value", str(hk or "unknown"))
        CANONICAL_HARD_OVERRIDE.labels(symbol=sym, kind=str(k)).inc()
    tc = getattr(risk, "degradation_transition_count", None)
    if tc is not None:
        try:
            CANONICAL_DEGRADATION_TRANSITION_COUNT.labels(symbol=sym).set(float(tc))
        except (TypeError, ValueError):
            pass

    if forecast_packet is not None:
        CANONICAL_OOD_SCORE.labels(symbol=sym).observe(float(forecast_packet.ood_score))
        fd = forecast_packet.forecast_diagnostics or {}
        tr = fd.get("trigger")
        if isinstance(tr, dict):
            for stage, key in (
                ("setup", "setup_valid"),
                ("pretrigger", "pretrigger_valid"),
                ("confirm", "trigger_valid"),
            ):
                passed = bool(tr.get(key))
                CANONICAL_TRIGGER_STAGE.labels(symbol=sym, stage=stage, passed=str(passed)).inc()
            if bool(tr.get("missed_move_flag")):
                CANONICAL_TRIGGER_MISSED_MOVE.labels(symbol=sym).inc()
            lat = tr.get("setup_to_confirm_latency_ms")
            if lat is not None:
                try:
                    CANONICAL_TRIGGER_SETUP_TO_CONFIRM_LATENCY_MS.labels(symbol=sym).observe(float(lat))
                except (TypeError, ValueError):
                    pass
        au = fd.get("auction")
        if isinstance(au, dict):
            sel = au.get("selected_score")
            if sel is not None:
                try:
                    CANONICAL_AUCTION_SCORE.labels(symbol=sym).observe(float(sel))
                except (TypeError, ValueError):
                    pass
            if au.get("selected_symbol") is None and au.get("records"):
                CANONICAL_AUCTION_SUPPRESSED.labels(symbol=sym).inc()
            ts = au.get("top_n_saturation")
            if ts is not None:
                try:
                    CANONICAL_AUCTION_TOP_N_SATURATION.labels(symbol=sym).observe(float(ts))
                except (TypeError, ValueError):
                    pass
            ce = au.get("candidates_evaluated")
            if ce is not None:
                try:
                    CANONICAL_AUCTION_CANDIDATES_EVALUATED.labels(symbol=sym).observe(float(ce))
                except (TypeError, ValueError):
                    pass

    sm = getattr(risk, "canonical_size_multiplier", None)
    if sm is not None:
        CANONICAL_SIZE_MULTIPLIER.labels(symbol=sym).observe(float(sm))

    rs = getattr(risk, "last_risk_sizing", None)
    if isinstance(rs, dict):
        fn = rs.get("final_notional_usd")
        if fn is not None:
            try:
                CANONICAL_FINAL_NOTIONAL_USD.labels(symbol=sym).observe(float(fn))
            except (TypeError, ValueError):
                pass

    age = getattr(risk, "data_age_seconds", None)
    if age is not None:
        try:
            CANONICAL_DATA_AGE_SECONDS.labels(symbol=sym).observe(float(age))
        except (TypeError, ValueError):
            pass

    cs = carry_sleeve
    if cs is None and risk is not None:
        cs = getattr(risk, "carry_sleeve_last", None)
    if isinstance(cs, dict):
        try:
            active = 1.0 if bool(cs.get("active")) else 0.0
            CANONICAL_CARRY_SLEEVE_ACTIVE.labels(symbol=sym).set(active)
            if cs.get("target_notional_usd") is not None:
                CANONICAL_CARRY_TARGET_NOTIONAL_USD.labels(symbol=sym).observe(
                    float(cs["target_notional_usd"])
                )
            if cs.get("funding_signal") is not None:
                CANONICAL_CARRY_FUNDING_SIGNAL.labels(symbol=sym).observe(float(cs["funding_signal"]))
            if cs.get("trigger_confidence") is not None:
                CANONICAL_CARRY_TRIGGER_CONFIDENCE.labels(symbol=sym).observe(
                    float(cs["trigger_confidence"])
                )
            if cs.get("decision_quality") is not None:
                CANONICAL_CARRY_DECISION_QUALITY.labels(symbol=sym).observe(float(cs["decision_quality"]))
            for rc in cs.get("reason_codes") or []:
                CANONICAL_CARRY_REASON.labels(symbol=sym, reason=str(rc)).inc()
            if bool(cs.get("directional_blocked")):
                CANONICAL_CARRY_DIRECTIONAL_SUPPRESSION.labels(symbol=sym).inc()
        except (TypeError, ValueError):
            pass

    record_calibration_and_drift_from_tick(
        symbol=sym,
        risk=risk,
        forecast_packet=forecast_packet,
        feature_row=feature_row,
    )

    fr = feature_row or {}
    try:
        CANONICAL_OPTIONS_CONTEXT_AVAILABLE.labels(symbol=sym).set(
            float(fr.get("options_context_available", 0.0))
        )
        CANONICAL_STABLECOIN_FLOW_AVAILABLE.labels(symbol=sym).set(
            float(fr.get("stablecoin_flow_available", 0.0))
        )
        CANONICAL_OPTIONS_CONTEXT_FALLBACK.labels(symbol=sym).set(
            float(fr.get("options_context_fallback_active", 0.0))
        )
        CANONICAL_STABLECOIN_FLOW_FALLBACK.labels(symbol=sym).set(
            float(fr.get("stablecoin_flow_proxy_fallback_active", fr.get("stablecoin_flow_fallback_active", 0.0)))
        )
    except (TypeError, ValueError):
        pass

    dr = getattr(risk, "last_decision_record", None)
    if isinstance(dr, dict):
        diag = dr.get("diagnostics") or {}
        eg = diag.get("execution_guidance_preview")
        if isinstance(eg, dict):
            st = eg.get("preferred_execution_style")
            if st is not None:
                CANONICAL_EXECUTION_STYLE.labels(symbol=sym, style=str(st)).inc()
            ecg = eg.get("execution_confidence")
            if ecg is not None:
                try:
                    CANONICAL_EXECUTION_GUIDANCE_CONFIDENCE.labels(symbol=sym).observe(float(ecg))
                except (TypeError, ValueError):
                    pass


def set_active_config_version(version: str) -> None:
    """Set info gauge for active config (call from startup or settings load)."""
    v = (version or "unknown").strip() or "unknown"
    CANONICAL_ACTIVE_CONFIG_VERSION.labels(version=v).set(1.0)


_config_gauge_done = False


def maybe_set_config_version_from_engine(risk_engine: Any) -> None:
    """Once per process, stamp active config version from RiskEngine settings."""
    global _config_gauge_done
    if _config_gauge_done:
        return
    try:
        s = getattr(risk_engine, "_settings", None)
        if s is None:
            return
        cv = s.canonical.metadata.config_version
        set_active_config_version(str(cv))
        _config_gauge_done = True
    except Exception:
        pass

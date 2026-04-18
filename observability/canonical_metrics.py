"""APEX canonical monitoring metric families (FB-CAN-010).

Maps domains in docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md
to Prometheus instruments. Call :func:`record_canonical_post_tick` once per decision cycle.
"""

from __future__ import annotations

from typing import Any

from prometheus_client import Counter, Gauge, Histogram

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
) -> None:
    """Record canonical metrics from one `run_decision_tick` completion."""
    sym = symbol or "unknown"

    apex = getattr(regime, "apex", None)
    if apex is not None:
        CANONICAL_REGIME_CONFIDENCE.labels(symbol=sym).observe(float(apex.regime_confidence))
        CANONICAL_HEAT_SCORE.labels(symbol=sym).observe(float(apex.heat_score))
        CANONICAL_NOVELTY.labels(symbol=sym).observe(float(apex.novelty))
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

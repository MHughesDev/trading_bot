"""APEX canonical configuration models (FB-CAN-003).

See docs/Human Provided Specs/new_specs/canonical/APEX_Canonical_Configuration_Spec_v1_0.md.
Runtime uses a versioned bundle: metadata + per-domain dicts. Legacy flat AppSettings remains
the operational source until each domain is wired; :func:`synthesize_canonical_from_legacy`
projects current settings into the canonical shape for replay/version stamping.
"""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

EnvironmentScope = Literal["research", "simulation", "shadow", "live", "unspecified"]


class CanonicalMetadata(BaseModel):
    """Global metadata and versioning (APEX spec §4)."""

    model_config = ConfigDict(extra="ignore")

    config_version: str = "1.0.0"
    config_name: str = "default"
    created_at: str = Field(
        default_factory=lambda: datetime.now(timezone.utc).replace(microsecond=0).isoformat()
    )
    created_by: str = "system"
    parent_config_version: str | None = None
    environment_scope: EnvironmentScope = "unspecified"
    notes: str = ""
    logic_version: str | None = None
    enabled_feature_families: list[str] = Field(default_factory=list)


class CanonicalDomains(BaseModel):
    """Per-domain configuration bags; keys follow APEX domain list (spec §3).

    Values are JSON-serializable dicts so we can evolve fields without breaking loaders.
    """

    model_config = ConfigDict(extra="allow")

    signal_confidence: dict[str, Any] = Field(default_factory=dict)
    state_safety_degradation: dict[str, Any] = Field(default_factory=dict)
    regime: dict[str, Any] = Field(default_factory=dict)
    forecast_calibration: dict[str, Any] = Field(default_factory=dict)
    trigger: dict[str, Any] = Field(default_factory=dict)
    auction: dict[str, Any] = Field(default_factory=dict)
    risk_sizing: dict[str, Any] = Field(default_factory=dict)
    execution: dict[str, Any] = Field(default_factory=dict)
    memory_adaptation: dict[str, Any] = Field(default_factory=dict)
    carry: dict[str, Any] = Field(default_factory=dict)
    monitoring: dict[str, Any] = Field(default_factory=dict)
    replay: dict[str, Any] = Field(default_factory=dict)
    feature_families: dict[str, Any] = Field(default_factory=dict)


class CanonicalRuntimeConfig(BaseModel):
    """Immutable logical config for stamping runs and replay (metadata + domains)."""

    model_config = ConfigDict(extra="ignore")

    metadata: CanonicalMetadata
    domains: CanonicalDomains


def synthesize_canonical_from_legacy(settings: Any) -> CanonicalRuntimeConfig:
    """Project current AppSettings into canonical domain dicts (compatibility layer)."""
    from app.config.settings import AppSettings

    if not isinstance(settings, AppSettings):
        raise TypeError("synthesize_canonical_from_legacy expects AppSettings")
    meta = CanonicalMetadata(
        config_version="1.0.0",
        config_name="legacy-app-settings-projection",
        notes="Synthesized from flat AppSettings / default.yaml until domains are fully wired.",
        environment_scope="unspecified",
        logic_version=None,
        enabled_feature_families=[
            "market_data",
            "features",
            "routing",
            "risk",
            "backtesting",
            "execution",
        ],
    )
    domains = CanonicalDomains(
        signal_confidence={
            "projection": "legacy",
            "features_return_windows": list(settings.features_return_windows),
            "features_volatility_windows": list(settings.features_volatility_windows),
        },
        state_safety_degradation={"projection": "legacy"},
        regime={"projection": "legacy"},
        forecast_calibration={
            "projection": "legacy",
            "models_forecaster_checkpoint_id": settings.models_forecaster_checkpoint_id,
            "models_torch_device": settings.models_torch_device,
        },
        trigger={"projection": "legacy"},
        auction={
            "projection": "legacy",
            "routing_spread_trade_max_bps": settings.routing_spread_trade_max_bps,
            "routing_forecast_strength_min": settings.routing_forecast_strength_min,
            "routing_score_scalping_forecast": settings.routing_score_scalping_forecast,
            "routing_score_intraday_forecast": settings.routing_score_intraday_forecast,
            "routing_score_swing_forecast": settings.routing_score_swing_forecast,
            "routing_spread_penalty_per_bp": settings.routing_spread_penalty_per_bp,
        },
        risk_sizing={
            "projection": "legacy",
            "max_total_exposure_usd": settings.risk_max_total_exposure_usd,
            "max_per_symbol_usd": settings.risk_max_per_symbol_usd,
            "max_drawdown_pct": settings.risk_max_drawdown_pct,
            "max_spread_bps": settings.risk_max_spread_bps,
            "stale_data_seconds": settings.risk_stale_data_seconds,
        },
        execution={
            "projection": "legacy",
            "execution_mode": settings.execution_mode,
            "execution_live_adapter": settings.execution_live_adapter,
            "execution_paper_adapter": settings.execution_paper_adapter,
            "portfolio_mark_price_source_paper": settings.portfolio_mark_price_source_paper,
            "portfolio_mark_price_source_live": settings.portfolio_mark_price_source_live,
        },
        memory_adaptation={
            "projection": "legacy",
            "memory_qdrant_collection": settings.memory_qdrant_collection,
            "memory_top_k": settings.memory_top_k,
        },
        carry={
            "projection": "legacy",
            "carry_enabled": False,
            "carry_activation_requires_directional_neutrality": True,
            "carry_max_exposure_usd": 5000.0,
            "carry_funding_threshold": 0.35,
            "carry_independent_risk_multiplier": 0.35,
            "carry_attribution_isolation_required": True,
            "carry_low_directional_trigger_confidence": 0.15,
        },
        monitoring={
            "projection": "legacy",
            "observability_log_level": settings.observability_log_level,
        },
        replay={
            "projection": "legacy",
            "backtesting_slippage_bps": settings.backtesting_slippage_bps,
            "backtesting_fee_bps": settings.backtesting_fee_bps,
            "backtesting_initial_cash_usd": settings.backtesting_initial_cash_usd,
            "backtesting_rng_seed": settings.backtesting_rng_seed,
        },
        feature_families={
            "projection": "legacy",
            "market_data_symbols": list(settings.market_data_symbols),
            "market_data_bar_interval_seconds": settings.market_data_bar_interval_seconds,
        },
    )
    return CanonicalRuntimeConfig(metadata=meta, domains=domains)


def _deep_merge_base(a: dict[str, Any], b: dict[str, Any]) -> dict[str, Any]:
    out = dict(a)
    for k, v in b.items():
        if k in out and isinstance(out[k], dict) and isinstance(v, dict):
            out[k] = _deep_merge_base(out[k], v)
        else:
            out[k] = v
    return out


def parse_canonical_from_yaml_fragment(raw: dict[str, Any] | None) -> CanonicalRuntimeConfig | None:
    """Parse ``apex_canonical`` top-level YAML section."""
    if not raw:
        return None
    meta = raw.get("metadata") or {}
    dom = raw.get("domains") or {}
    return CanonicalRuntimeConfig(
        metadata=CanonicalMetadata.model_validate(meta),
        domains=CanonicalDomains.model_validate(dom),
    )


def merge_canonical(
    base: CanonicalRuntimeConfig,
    override: CanonicalRuntimeConfig,
) -> CanonicalRuntimeConfig:
    """Merge two configs: override metadata fields that are set; deep-merge domains."""
    md_base = base.metadata.model_dump()
    md_ov = override.metadata.model_dump(exclude_none=True)
    merged_meta = CanonicalMetadata.model_validate(_deep_merge_base(md_base, md_ov))
    d_base = base.domains.model_dump()
    d_ov = override.domains.model_dump()
    merged_dom = CanonicalDomains.model_validate(_deep_merge_base(d_base, d_ov))
    return CanonicalRuntimeConfig(metadata=merged_meta, domains=merged_dom)


def resolve_canonical_config(
    settings: Any,
    yaml_cfg: dict[str, Any] | None,
) -> CanonicalRuntimeConfig:
    """Final canonical bundle: YAML ``apex_canonical`` merged over legacy synthesis."""
    synthesized = synthesize_canonical_from_legacy(settings)
    if not yaml_cfg:
        return synthesized
    fragment = yaml_cfg.get("apex_canonical")
    if not fragment or not isinstance(fragment, dict):
        return synthesized
    parsed = parse_canonical_from_yaml_fragment(fragment)
    if parsed is None:
        return synthesized
    return merge_canonical(synthesized, parsed)

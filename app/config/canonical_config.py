"""APEX canonical configuration models (FB-CAN-003).

See docs/Human Provided Specs/new_specs/canonical/APEX_Canonical_Configuration_Spec_v1_0.md.
Runtime uses a versioned bundle: metadata + per-domain dicts. Flat :class:`~app.config.settings.AppSettings`
(from ``default.yaml`` + ``NM_*``) is the operational source; :func:`synthesize_canonical_from_app_settings`
projects it into the canonical shape for replay/version stamping. YAML ``apex_canonical`` deep-merges on top
via :func:`resolve_canonical_config` (FB-CAN-060 removed migration-only ``projection: legacy`` markers).
"""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

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
    notes: str = "No additional notes."
    logic_version: str | None = None
    enabled_feature_families: list[str] = Field(default_factory=list)

    @field_validator("enabled_feature_families", mode="after")
    @classmethod
    def _non_empty_feature_families(cls, v: list[str]) -> list[str]:
        if not v:
            raise ValueError("metadata.enabled_feature_families must be non-empty")
        return v

    @field_validator("config_version", mode="before")
    @classmethod
    def _coerce_config_version_str(cls, v: Any) -> Any:
        if v is None:
            return v
        s = str(v).strip()
        if not s:
            raise ValueError("metadata.config_version must be non-empty")
        return s

    @field_validator(
        "config_name",
        "created_at",
        "created_by",
        "notes",
        mode="before",
    )
    @classmethod
    def _strip_nonempty_strings(cls, v: Any) -> Any:
        if v is None:
            return v
        if isinstance(v, str) and not v.strip():
            raise ValueError("must be non-empty when provided")
        return v


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
    shadow_comparison: dict[str, Any] = Field(default_factory=dict)
    post_release_probation: dict[str, Any] = Field(
        default_factory=dict,
        description="Live post-release probation windows and abort thresholds (FB-CAN-069).",
    )
    runtime_cutover: dict[str, Any] = Field(
        default_factory=dict,
        description="Cutover / migration-shadow flags (FB-CAN-059); see default.yaml.",
    )


class CanonicalRuntimeConfig(BaseModel):
    """Immutable logical config for stamping runs and replay (metadata + domains)."""

    model_config = ConfigDict(extra="ignore")

    metadata: CanonicalMetadata
    domains: CanonicalDomains


def synthesize_canonical_from_app_settings(settings: Any) -> CanonicalRuntimeConfig:
    """Project current AppSettings into canonical domain dicts (baseline before YAML overlay)."""
    from app.config.settings import AppSettings

    if not isinstance(settings, AppSettings):
        raise TypeError("synthesize_canonical_from_app_settings expects AppSettings")
    meta = CanonicalMetadata(
        config_version="1.0.0",
        config_name="app-settings-synthesis",
        created_at=datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        created_by="app-settings-synthesis",
        notes="Synthesized from AppSettings + default.yaml; merged with apex_canonical from YAML when present.",
        environment_scope="unspecified",
        logic_version=None,
        enabled_feature_families=[
            "market_data",
            "features",
            "routing",
            "risk",
            "backtesting",
            "execution",
            "market_microstructure",
            "execution_feedback",
            "novelty",
            "heat_components",
        ],
    )
    domains = CanonicalDomains(
        signal_confidence={
            "source": "app_settings",
            "features_return_windows": list(settings.features_return_windows),
            "features_volatility_windows": list(settings.features_volatility_windows),
            "note": "Per-family params live in YAML apex_canonical.domains.signal_confidence (FB-CAN-032).",
        },
        state_safety_degradation={"source": "app_settings"},
        regime={"source": "app_settings"},
        forecast_calibration={
            "source": "app_settings",
            "models_forecaster_checkpoint_id": settings.models_forecaster_checkpoint_id,
            "models_torch_device": settings.models_torch_device,
        },
        trigger={"source": "app_settings"},
        auction={
            "source": "app_settings",
            "routing_spread_trade_max_bps": settings.routing_spread_trade_max_bps,
            "routing_forecast_strength_min": settings.routing_forecast_strength_min,
            "routing_score_scalping_forecast": settings.routing_score_scalping_forecast,
            "routing_score_intraday_forecast": settings.routing_score_intraday_forecast,
            "routing_score_swing_forecast": settings.routing_score_swing_forecast,
            "routing_spread_penalty_per_bp": settings.routing_spread_penalty_per_bp,
        },
        risk_sizing={
            "source": "app_settings",
            "max_total_exposure_usd": settings.risk_max_total_exposure_usd,
            "max_per_symbol_usd": settings.risk_max_per_symbol_usd,
            "max_drawdown_pct": settings.risk_max_drawdown_pct,
            "max_spread_bps": settings.risk_max_spread_bps,
            "stale_data_seconds": settings.risk_stale_data_seconds,
        },
        execution={
            "source": "app_settings",
            "execution_mode": settings.execution_mode,
            "execution_live_adapter": settings.execution_live_adapter,
            "execution_paper_adapter": settings.execution_paper_adapter,
            "portfolio_mark_price_source_paper": settings.portfolio_mark_price_source_paper,
            "portfolio_mark_price_source_live": settings.portfolio_mark_price_source_live,
        },
        memory_adaptation={
            "source": "app_settings",
            "memory_qdrant_collection": settings.memory_qdrant_collection,
            "memory_top_k": settings.memory_top_k,
        },
        carry={
            "source": "app_settings",
            "carry_enabled": False,
            "carry_activation_requires_directional_neutrality": True,
            "carry_max_exposure_usd": 5000.0,
            "carry_funding_threshold": 0.35,
            "carry_independent_risk_multiplier": 0.35,
            "carry_attribution_isolation_required": True,
            "carry_low_directional_trigger_confidence": 0.15,
        },
        monitoring={
            "source": "app_settings",
            "observability_log_level": settings.observability_log_level,
        },
        replay={
            "source": "app_settings",
            "backtesting_slippage_bps": settings.backtesting_slippage_bps,
            "backtesting_fee_bps": settings.backtesting_fee_bps,
            "backtesting_initial_cash_usd": settings.backtesting_initial_cash_usd,
            "backtesting_rng_seed": settings.backtesting_rng_seed,
        },
        shadow_comparison={
            "source": "app_settings",
            "note": "Override with apex_canonical.domains.shadow_comparison (FB-CAN-038).",
        },
        post_release_probation={
            "source": "app_settings",
            "note": "Override with apex_canonical.domains.post_release_probation (FB-CAN-069).",
        },
        runtime_cutover={
            "source": "app_settings",
            "phase": "canonical_active",
            "migration_shadow_allowed": False,
            "note": "Set migration_shadow_allowed true only when enabling runtime bridge in_process (FB-CAN-059).",
        },
        feature_families={
            "source": "app_settings",
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
    # exclude_defaults so partial YAML fragments do not wipe lists/strings with model defaults (FB-CAN-061).
    md_ov = override.metadata.model_dump(exclude_none=True, exclude_defaults=True)
    merged_meta = CanonicalMetadata.model_validate(_deep_merge_base(md_base, md_ov))
    d_base = base.domains.model_dump()
    d_ov = override.domains.model_dump()
    merged_dom = CanonicalDomains.model_validate(_deep_merge_base(d_base, d_ov))
    return CanonicalRuntimeConfig(metadata=merged_meta, domains=merged_dom)


def resolve_canonical_config(
    settings: Any,
    yaml_cfg: dict[str, Any] | None,
) -> CanonicalRuntimeConfig:
    """Final canonical bundle: YAML ``apex_canonical`` merged over AppSettings synthesis."""
    from app.config.canonical_metadata_validation import validate_canonical_runtime_metadata

    synthesized = synthesize_canonical_from_app_settings(settings)
    if not yaml_cfg:
        out = synthesized
    else:
        fragment = yaml_cfg.get("apex_canonical")
        if not fragment or not isinstance(fragment, dict):
            out = synthesized
        else:
            parsed = parse_canonical_from_yaml_fragment(fragment)
            if parsed is None:
                out = synthesized
            else:
                out = merge_canonical(synthesized, parsed)
    validate_canonical_runtime_metadata(
        out.metadata,
        execution_mode=getattr(settings, "execution_mode", "paper"),
    )
    return out

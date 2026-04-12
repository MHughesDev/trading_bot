"""Per-asset model manifest (FB-AP-001) — binds one canonical symbol to forecaster + RL artifact paths."""

from __future__ import annotations

from pydantic import BaseModel, Field, field_validator


class AssetModelManifest(BaseModel):
    """
    Versioned contract for **one** tradable symbol's model artifacts.

    ``canonical_symbol`` must match the symbol used in API/runtime when loading this manifest;
    callers should use :func:`app.runtime.asset_model_registry.validate_manifest_symbol`.
    """

    schema_version: str = Field(default="1", description="Manifest format version")
    canonical_symbol: str = Field(
        ...,
        min_length=1,
        description="Single symbol this manifest applies to (e.g. BTC-USD)",
    )
    forecaster_weights_path: str | None = None
    forecaster_conformal_state_path: str | None = None
    forecaster_config_path: str | None = None
    forecaster_torch_path: str | None = None
    policy_mlp_path: str | None = None
    policy_checkpoint_path: str | None = None
    runtime_instance_id: str | None = None
    forecaster_last_trained_at: str | None = Field(
        default=None,
        description="ISO-8601 timestamp of last forecaster train for this asset",
    )
    rl_last_trained_at: str | None = Field(
        default=None,
        description="ISO-8601 timestamp of last RL train for this asset",
    )

    @field_validator("canonical_symbol")
    @classmethod
    def _strip_symbol(cls, v: str) -> str:
        s = v.strip()
        if not s:
            raise ValueError("canonical_symbol cannot be empty")
        return s

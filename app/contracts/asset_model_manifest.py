"""Per-asset model manifest (FB-AP-001): versioned binding of one canonical symbol to artifact paths."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field, field_validator


class ForecasterArtifactPaths(BaseModel):
    """NPZ / torch / conformal paths for the hot-path forecaster."""

    weights_npz_path: str | None = None
    torch_path: str | None = None
    conformal_state_path: str | None = None
    checkpoint_id: str | None = None


class RLArtifactPaths(BaseModel):
    """Policy / RL artifact paths (per-asset manifest; FB-AP epic)."""

    policy_mlp_path: str | None = None


class LastTrainedMeta(BaseModel):
    """Optional training timestamps (ISO 8601 strings)."""

    forecaster_utc: str | None = None
    rl_utc: str | None = None


class AssetModelManifestV1(BaseModel):
    """
    Versioned per-asset manifest: exactly one traded symbol and its artifact locations.

    Validation on load: `symbol` must match the decision tick symbol when manifest binding is active.
    """

    schema_version: Literal[1] = 1
    manifest_id: str = Field(min_length=1, description="Stable id (e.g. UUID) for operator logs")
    symbol: str = Field(min_length=1)
    forecaster: ForecasterArtifactPaths = Field(default_factory=ForecasterArtifactPaths)
    rl: RLArtifactPaths = Field(default_factory=RLArtifactPaths)
    runtime_instance_id: str | None = None
    last_trained: LastTrainedMeta = Field(default_factory=LastTrainedMeta)

    @field_validator("symbol")
    @classmethod
    def _strip_symbol(cls, v: str) -> str:
        s = v.strip()
        if not s:
            raise ValueError("symbol must be non-empty")
        return s

    def assert_matches_decision_symbol(self, decision_symbol: str) -> None:
        """Refuse silent cross-symbol reuse: manifest symbol must equal the tick symbol (after strip)."""
        ds = decision_symbol.strip()
        if self.symbol != ds:
            raise ValueError(
                f"manifest symbol {self.symbol!r} does not match decision symbol {ds!r} "
                f"(manifest_id={self.manifest_id!r})"
            )

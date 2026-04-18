"""Post-release live probation policy (FB-CAN-069).

Validated from ``apex_canonical.domains.post_release_probation`` in default.yaml.
See APEX Config Management spec §13 (probation windows).
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field


class PostReleaseProbationThresholds(BaseModel):
    """Risk-quality proxies during early live window (0–1 scales)."""

    model_config = ConfigDict(extra="ignore")

    edge_erosion_p95_max: float = Field(0.35, ge=0.0, le=1.0)
    feature_drift_penalty_p95_max: float = Field(0.55, ge=0.0, le=1.0)
    trigger_false_positive_memory_p95_max: float = Field(0.65, ge=0.0, le=1.0)


class PostReleaseProbationWindows(BaseModel):
    """Time bounds after ``approved_at`` on the active-live release candidate."""

    model_config = ConfigDict(extra="ignore")

    active_hours: float = Field(48.0, ge=1.0, le=168.0)
    cooldown_hours: float = Field(72.0, ge=0.0, le=336.0)
    total_window_hours: float = Field(120.0, ge=1.0, le=720.0)


class PostReleaseProbationPolicy(BaseModel):
    """Elevated monitoring + automatic abort recommendation after live promotion."""

    model_config = ConfigDict(extra="ignore")

    enabled: bool = True
    windows: PostReleaseProbationWindows = Field(default_factory=PostReleaseProbationWindows)
    thresholds: PostReleaseProbationThresholds = Field(default_factory=PostReleaseProbationThresholds)
    sample_window_ticks: int = Field(256, ge=32, le=50_000)


def validate_post_release_probation_domain(raw: dict[str, object] | None) -> list[str]:
    """Return validation error strings for CI gates."""
    if raw is None:
        return []
    if not isinstance(raw, dict):
        return ["post_release_probation must be a mapping"]
    try:
        PostReleaseProbationPolicy.model_validate(raw)
    except Exception as e:
        return [f"post_release_probation: {e}"]
    return []


def probation_policy_from_settings(settings: object) -> PostReleaseProbationPolicy:
    """Read policy from resolved canonical bundle, or defaults."""
    try:
        dom = getattr(settings, "canonical", None)
        if dom is None:
            return PostReleaseProbationPolicy()
        bag = getattr(dom, "domains", None)
        if bag is None:
            return PostReleaseProbationPolicy()
        raw = getattr(bag, "post_release_probation", None)
        if isinstance(raw, dict) and raw:
            return PostReleaseProbationPolicy.model_validate(raw)
    except Exception:
        pass
    return PostReleaseProbationPolicy()

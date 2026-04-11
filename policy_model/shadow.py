"""Policy shadow / canary: compare policies without execution (FB-PL-PG3)."""

from __future__ import annotations

from collections.abc import Callable

from policy_model.objects import PolicyAction, PolicyObservation

try:
    from observability.forecaster_metrics import POLICY_SHADOW_DELTA
except ImportError:  # pragma: no cover
    POLICY_SHADOW_DELTA = None


def shadow_compare_actions(
    obs: PolicyObservation,
    primary: Callable[[PolicyObservation], PolicyAction],
    shadow: Callable[[PolicyObservation], PolicyAction],
    *,
    record_metric: bool = True,
) -> dict[str, float]:
    """Returns target_exposure for primary and shadow plus absolute delta."""
    a_p = primary(obs).target_exposure
    a_s = shadow(obs).target_exposure
    delta = abs(a_p - a_s)
    if record_metric and POLICY_SHADOW_DELTA:
        POLICY_SHADOW_DELTA.observe(delta)
    return {"primary": a_p, "shadow": a_s, "abs_delta": delta}

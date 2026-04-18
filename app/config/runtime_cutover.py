"""Runtime cutover guard (FB-CAN-059).

Prevents **mixed activation** of the in-process live decision path and the
microservice **runtime bridge** shadow handoff unless the operator explicitly opts
into **migration shadow** mode in canonical config.

See ``apex_canonical.domains.runtime_cutover`` in ``default.yaml``.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from app.config.settings import AppSettings


def _runtime_cutover_dict(settings: AppSettings) -> dict[str, Any]:
    dom = settings.canonical.domains
    raw = getattr(dom, "runtime_cutover", None)
    if isinstance(raw, dict):
        return raw
    return {}


def validate_runtime_cutover(settings: AppSettings) -> None:
    """
    Fail fast when runtime bridge is enabled in **in_process** mode without
    ``migration_shadow_allowed: true`` — that combination duplicates decision/proposal
    flow (live ``run_decision_tick`` + bridge consumer pipeline).

    Call from :func:`load_settings` and live runtime entrypoints.
    """
    rc = _runtime_cutover_dict(settings)
    migration = bool(rc.get("migration_shadow_allowed", False))
    if not settings.microservices_runtime_bridge_enabled:
        return
    if settings.microservices_execution_gateway_mode != "in_process":
        return
    if migration:
        return
    raise RuntimeError(
        "Runtime cutover guard (FB-CAN-059): microservices_runtime_bridge_enabled=true with "
        "microservices_execution_gateway_mode=in_process requires "
        "apex_canonical.domains.runtime_cutover.migration_shadow_allowed=true "
        "(migration shadow mode). Otherwise disable the bridge or set execution_gateway_mode "
        "to external so only one execution/decision path is active."
    )

"""Canonical partial-fill reconciliation (FB-CAN-048).

Bridges :func:`execution.execution_logic.reconcile_partial_fill` with typed records and
cancel/replace policy hints for replay and gateways.
"""

from __future__ import annotations

from typing import Any

from app.config.settings import AppSettings
from app.contracts.partial_fill import PartialFillReconciliation
from execution.execution_logic import reconcile_partial_fill


def _execution_domain(settings: AppSettings | None) -> dict[str, Any]:
    if settings is None:
        return {}
    try:
        ex = settings.canonical.domains.execution
        return dict(ex) if isinstance(ex, dict) else {}
    except Exception:
        return {}


def reconcile_partial_fill_record(
    *,
    intended_qty: float,
    fill_ratio: float,
    remaining_edge: float,
    execution_confidence_realized: float,
    settings: AppSettings | None = None,
    domain: dict[str, Any] | None = None,
) -> PartialFillReconciliation:
    """Build :class:`PartialFillReconciliation` from deterministic spec logic."""
    dom = domain if domain is not None else _execution_domain(settings)
    min_rem = float(dom.get("partial_fill_min_remaining_fraction", 0.02))
    min_edge = float(dom.get("minimum_tradeable_edge", 0.0015))
    low_floor = float(dom.get("partial_fill_low_execution_floor", 0.22))

    fr = max(0.0, min(1.0, float(fill_ratio)))
    iq = max(0.0, float(intended_qty))
    filled = iq * fr
    residual = max(0.0, iq - filled)

    outcome = reconcile_partial_fill(
        intended_qty=iq,
        fill_ratio=fr,
        remaining_edge=float(remaining_edge),
        min_remaining_fraction=min_rem,
        minimum_tradeable_edge=min_edge,
        execution_confidence_realized=float(execution_confidence_realized),
        low_execution_floor=low_floor,
    )

    if outcome == "done":
        policy = "none"
        codes = ["partial_fill_done"]
    elif outcome == "abandon":
        policy = "none"
        codes = ["partial_fill_abandon_residual_edge"]
    elif outcome == "pause_or_reduce":
        policy = "hold"
        codes = ["partial_fill_pause_low_exec_confidence"]
    else:
        policy = "reschedule_child"
        codes = ["partial_fill_continue_staggered"]

    if fr < 1.0 - 1e-12:
        codes.append(f"fill_ratio_{fr:.4f}")

    return PartialFillReconciliation(
        outcome=outcome,
        intended_qty=iq,
        filled_qty=filled,
        residual_qty=residual,
        fill_ratio=fr,
        remaining_edge=float(remaining_edge),
        execution_confidence_realized=float(execution_confidence_realized),
        cancel_replace_policy=policy,
        reason_codes=codes,
    )

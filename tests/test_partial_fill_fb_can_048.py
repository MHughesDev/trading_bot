"""FB-CAN-048 partial-fill reconciliation."""

from __future__ import annotations

from app.config.settings import AppSettings
from backtesting.replay_helpers import scaled_order_quantity_for_fill_ratio
from execution.partial_fill_reconcile import reconcile_partial_fill_record
from decimal import Decimal


def test_reconcile_partial_fill_record_done():
    r = reconcile_partial_fill_record(
        intended_qty=1.0,
        fill_ratio=0.99,
        remaining_edge=0.02,
        execution_confidence_realized=0.5,
        settings=AppSettings(),
    )
    assert r.outcome == "done"
    assert r.residual_qty < 0.05
    assert r.cancel_replace_policy == "none"


def test_reconcile_partial_fill_record_continue_staggered():
    r = reconcile_partial_fill_record(
        intended_qty=1.0,
        fill_ratio=0.4,
        remaining_edge=0.02,
        execution_confidence_realized=0.8,
        settings=AppSettings(),
    )
    assert r.outcome == "continue_staggered"
    assert r.cancel_replace_policy == "reschedule_child"


def test_scaled_qty_matches_fill_ratio():
    q = scaled_order_quantity_for_fill_ratio(Decimal("10"), 0.75)
    assert q == Decimal("7.5")

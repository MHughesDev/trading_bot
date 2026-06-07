"""ExecutionPlan — human policy spec §8.9."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class ExecutionPlan:
    required_delta_fraction: float
    required_delta_notional: float
    execution_mode: str
    max_child_order_size: float | None
    urgency: float
    skip_execution: bool
    skip_reasons: list[str]

"""APEX canonical replay / simulation contracts (FB-CAN-009).

See docs/Human Provided Specs/new_specs/canonical/APEX_Replay_and_Simulation_Interface_Spec_v1_0.md
§5–7, §10.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Any, Literal

from pydantic import BaseModel, Field


class ReplayMode(StrEnum):
    """Canonical replay modes (spec §5). FB-CAN-055: StrEnum for validation + typing."""

    HISTORICAL_NOMINAL = "historical_nominal"
    HISTORICAL_STRESS = "historical_stress"
    SYNTHETIC_FAULT_INJECTED = "synthetic_fault_injected"
    SHADOW_COMPARISON = "shadow_comparison"
    TRIGGER_DEBUG = "trigger_debug"
    EXECUTION_DEBUG = "execution_debug"


ExecutionModelProfile = Literal["optimistic", "baseline", "stress", "cascade_stress"]

ReplayEventFamily = Literal[
    "market_snapshot_event",
    "structural_signal_event",
    "safety_snapshot_event",
    "execution_feedback_event",
    "config_change_event",
    "fault_injection_event",
    "decision_output_event",
]


class ReplayRunContract(BaseModel):
    """Required fields for a replay run (spec §5)."""

    replay_run_id: str
    dataset_id: str = "default"
    config_version: str = "1.0.0"
    logic_version: str = "1.0.0"
    time_range_start: str | None = None
    time_range_end: str | None = None
    instrument_scope: list[str] = Field(default_factory=list)
    replay_mode: ReplayMode = ReplayMode.HISTORICAL_NOMINAL
    execution_model_profile: ExecutionModelProfile = "baseline"
    fault_injection_profile_id: str | None = Field(
        default=None,
        description="Named canonical profile from orchestration.fault_injection_profiles (FB-CAN-037); merged under fault_injection_profile.",
    )
    fault_injection_profile: dict[str, Any] = Field(default_factory=dict)
    seed: int | None = None


class ReplayEventEnvelope(BaseModel):
    """Single canonical replay event (spec §6)."""

    event_family: ReplayEventFamily
    replay_run_id: str
    symbol: str
    timestamp: Any = None
    payload: dict[str, Any] = Field(default_factory=dict)

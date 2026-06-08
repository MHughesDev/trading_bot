"""Canonical named fault-injection profiles for replay/simulation (FB-CAN-037).

Maps stable profile ids to :func:`backtesting.fault_injection.apply_fault_injection`
keyword dicts. Used by :class:`app.contracts.replay_events.ReplayRunContract` and
release evidence bundles so promotion can reference deterministic stress runs.

See APEX Replay spec §8.2–8.3 and Config Management spec (evidence packages).
"""

from __future__ import annotations

from typing import Any, Final, Literal

FaultProfileId = Literal[
    "market_data_outage",
    "stale_structural_feed",
    "spread_widening_stress",
    "book_imbalance_stress",
    "latency_stress",
    "execution_degradation",
    "liquidity_collapse",
]

# Deterministic profile payloads (merged into replay fault_injection_profile).
CANONICAL_FAULT_PROFILES: Final[dict[str, dict[str, Any]]] = {
    "market_data_outage": {
        "drop_feature_keys": ["rsi_14", "atr_14", "return_1"],
        "stale_data_seconds": 600.0,
        "venue_degradation_scale": 0.25,
    },
    "stale_structural_feed": {
        "stale_data_seconds": 420.0,
        "structural_freshness_scale": 0.35,
        "structural_reliability_scale": 0.4,
    },
    "spread_widening_stress": {
        "spread_widen_mult": 4.5,
    },
    "book_imbalance_stress": {
        "book_imbalance_bias": 0.92,
    },
    "latency_stress": {
        "latency_delay_seconds": 2.5,
    },
    "execution_degradation": {
        "execution_slippage_bps_add": 35.0,
        "execution_fill_ratio_mult": 0.55,
        "venue_degradation_scale": 0.2,
    },
    "liquidity_collapse": {
        "spread_widening_injection": True,
        "spread_widen_mult": 6.0,
        "liquidity_depth_scale": 0.08,
        "volume_burst_scale": 0.12,
    },
}


def list_canonical_fault_profile_ids() -> tuple[str, ...]:
    """Stable ordering for CLI / CI."""
    return tuple(sorted(CANONICAL_FAULT_PROFILES.keys()))


def resolve_fault_profile_dict(profile_id: str | None) -> dict[str, Any]:
    """Return merged fault dict for a named profile, or empty if unknown / None."""
    if not profile_id or not str(profile_id).strip():
        return {}
    pid = str(profile_id).strip()
    base = CANONICAL_FAULT_PROFILES.get(pid)
    return dict(base) if base is not None else {}


def merge_replay_fault_profile(
    *,
    fault_injection_profile_id: str | None,
    contract_profile: dict[str, Any] | None,
    override: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Merge named profile → contract dict → optional kwargs override (replay entrypoints)."""
    out: dict[str, Any] = {}
    out.update(resolve_fault_profile_dict(fault_injection_profile_id))
    out.update(dict(contract_profile or {}))
    if override:
        out.update(override)
    return out


def fault_stress_evidence_satisfied(
    *,
    fault_stress_run_ids: list[str],
    fault_profile_ids_satisfied: list[str],
) -> bool:
    """True when at least one stress run is recorded and all canonical profiles are listed."""
    if not fault_stress_run_ids:
        return False
    required = set(list_canonical_fault_profile_ids())
    got = {str(x).strip() for x in fault_profile_ids_satisfied if str(x).strip()}
    return required <= got

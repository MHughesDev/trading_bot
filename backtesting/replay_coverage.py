"""Canonical replay mode ↔ event-family coverage (FB-CAN-055).

Aligned with ``APEX_Replay_and_Simulation_Interface_Spec_v1_0.md`` §5–6: each
``replay_mode`` implies a minimum set of emitted ``event_family`` values when
``emit_canonical_events`` is enabled.
"""

from __future__ import annotations

from typing import Any

from app.contracts.replay_events import ReplayEventFamily, ReplayMode, ReplayRunContract

# Spec §6 — canonical event families (strings match ReplayEventFamily).
_CORE_PATH: frozenset[ReplayEventFamily] = frozenset(
    {
        "market_snapshot_event",
        "structural_signal_event",
        "safety_snapshot_event",
        "decision_output_event",
    }
)

# Minimum families that must appear at least once across the run (union of all bars).
_REQUIRED_FAMILIES_BY_MODE: dict[ReplayMode, frozenset[ReplayEventFamily]] = {
    ReplayMode.HISTORICAL_NOMINAL: _CORE_PATH,
    ReplayMode.HISTORICAL_STRESS: _CORE_PATH,
    ReplayMode.SYNTHETIC_FAULT_INJECTED: _CORE_PATH | frozenset({"fault_injection_event"}),
    ReplayMode.SHADOW_COMPARISON: _CORE_PATH,
    ReplayMode.TRIGGER_DEBUG: _CORE_PATH,
    ReplayMode.EXECUTION_DEBUG: _CORE_PATH | frozenset({"execution_feedback_event"}),
}


def _contract_has_fault_config(contract: ReplayRunContract) -> bool:
    if contract.fault_injection_profile_id and str(contract.fault_injection_profile_id).strip():
        return True
    fp = contract.fault_injection_profile
    return bool(fp)


def _row_had_trade(row: dict[str, Any]) -> bool:
    t = row.get("trade")
    if t is not None:
        return True
    sym = row.get("symbols")
    if isinstance(sym, dict):
        for payload in sym.values():
            if isinstance(payload, dict) and payload.get("trade") is not None:
                return True
    return False


def _any_trade_in_run(rows: list[dict[str, Any]]) -> bool:
    return any(_row_had_trade(r) for r in rows)


def collect_event_families_from_replay_rows(rows: list[dict[str, Any]]) -> set[str]:
    """Union of all ``event_family`` strings from per-row ``canonical_events``."""
    out: set[str] = set()
    for row in rows:
        evs = row.get("canonical_events")
        if isinstance(evs, list):
            for ev in evs:
                if isinstance(ev, dict) and ev.get("event_family"):
                    out.add(str(ev["event_family"]))
        syms = row.get("symbols")
        if isinstance(syms, dict):
            for payload in syms.values():
                if not isinstance(payload, dict):
                    continue
                evs2 = payload.get("canonical_events")
                if isinstance(evs2, list):
                    for ev in evs2:
                        if isinstance(ev, dict) and ev.get("event_family"):
                            out.add(str(ev["event_family"]))
    return out


def required_event_families_for_contract(contract: ReplayRunContract) -> frozenset[ReplayEventFamily]:
    """Required event families for ``contract.replay_mode`` (before trade-aware tweaks)."""
    mode = contract.replay_mode
    if isinstance(mode, str):
        mode = ReplayMode(mode)
    base = _REQUIRED_FAMILIES_BY_MODE.get(mode)
    if base is None:
        return frozenset()
    req = set(base)
    # Stress with an active fault profile must emit at least one fault_injection_event.
    if mode is ReplayMode.HISTORICAL_STRESS and _contract_has_fault_config(contract):
        req.add("fault_injection_event")
    return frozenset(req)


def validate_replay_event_family_coverage(
    rows: list[dict[str, Any]],
    contract: ReplayRunContract,
    *,
    emit_canonical_events: bool,
) -> tuple[bool, list[str]]:
    """
    Return (ok, reasons). Skips when ``emit_canonical_events`` is False.

    ``execution_debug`` drops the ``execution_feedback_event`` requirement when the
    run never produced a simulated trade (no execution path to observe).
    """
    if not emit_canonical_events or not rows:
        return True, []

    mode = contract.replay_mode
    if isinstance(mode, str):
        mode = ReplayMode(mode)

    required = set(required_event_families_for_contract(contract))
    if mode is ReplayMode.EXECUTION_DEBUG and not _any_trade_in_run(rows):
        required.discard("execution_feedback_event")

    present = collect_event_families_from_replay_rows(rows)
    missing = sorted(required - present)
    if not missing:
        return True, []

    return False, [
        f"replay_mode={mode.value!s} requires canonical event families {sorted(required)!r}; "
        f"missing after full run: {missing!r} (see APEX Replay spec §5–6; FB-CAN-055)"
    ]

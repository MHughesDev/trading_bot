"""Last-decision-record store for the control plane's governance view (FB-CAN-036).

Decoupled from the retired AI decision pipeline (FB-AP-XXX): the strategy-based runtime
(:mod:`app.runtime.strategy_decision_source`) doesn't build the old canonical ``DecisionRecord``
contract, so this store stays empty in normal operation — the endpoint simply reports that no
decision-record tick has run. Kept as a tiny standalone module (rather than importing from
``legacy/decision_pipeline``) so the runtime has zero dependency on the preserved snapshot.
"""

from __future__ import annotations

from typing import Any

_LAST_DECISION_RECORD: dict[str, Any] | None = None


def set_last_decision_record(record: dict[str, Any]) -> None:
    """Expose last tick for the control plane (single-process operator view)."""
    global _LAST_DECISION_RECORD
    _LAST_DECISION_RECORD = record


def get_last_decision_record() -> dict[str, Any] | None:
    return _LAST_DECISION_RECORD

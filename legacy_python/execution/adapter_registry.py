"""Supported execution venues (FB-N1 — central registry for operator / API visibility).

Hot path still uses **one** paper adapter (Alpaca) and **one** live adapter (Coinbase) per V1 spec.
Additional venues require new adapter modules and factory wiring — see [`docs/DEFERRED_ROADMAP.MD`](../docs/DEFERRED_ROADMAP.MD).
"""

from __future__ import annotations

import os
from typing import Any

from app.config.settings import AppSettings

# Keys match `execution_paper_adapter` / `execution_live_adapter` in settings (lowercase).
SUPPORTED_PAPER_ADAPTERS: frozenset[str] = frozenset({"alpaca"})
SUPPORTED_LIVE_ADAPTERS: frozenset[str] = frozenset({"coinbase"})

# Optional overrides via NM_EXECUTION_ADAPTER (see execution/router.py)
KNOWN_EXECUTION_ADAPTER_OVERRIDES: frozenset[str] = frozenset(
    {"stub", "mock_alpaca_paper", "mock_alpaca"}
)


def supported_adapters_for_settings(settings: AppSettings) -> dict[str, Any]:
    """Payload for control plane / operators — not a guarantee of runtime adapter instance."""
    paper = settings.execution_paper_adapter.strip().lower()
    live = settings.execution_live_adapter.strip().lower()
    env_override = os.getenv("NM_EXECUTION_ADAPTER", "").strip().lower()
    return {
        "paper_adapter_configured": paper,
        "live_adapter_configured": live,
        "paper_supported": paper in SUPPORTED_PAPER_ADAPTERS,
        "live_supported": live in SUPPORTED_LIVE_ADAPTERS,
        "supported_paper": sorted(SUPPORTED_PAPER_ADAPTERS),
        "supported_live": sorted(SUPPORTED_LIVE_ADAPTERS),
        "nm_execution_adapter_override": env_override or None,
        "override_recognized": env_override in KNOWN_EXECUTION_ADAPTER_OVERRIDES if env_override else False,
        "note": "V1: single venue per mode; multi-exchange routing is backlog FB-N1 (see docs/DEFERRED_ROADMAP.MD).",
    }

"""
Initial policy MLP weights for per-asset init (FB-AP-011).

Materializes :class:`policy_model.policy.mlp_actor.MultiBranchMLPPolicy` with a **deterministic**
seed derived from ``symbol`` and ``job_id``, saves ``policy_mlp.npz`` under ``<run_dir>/policy/``.
"""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


def _policy_seed(symbol: str, job_id: str) -> int:
    raw = hashlib.sha256(f"{symbol.strip()}|{job_id}".encode()).digest()
    return int.from_bytes(raw[:4], "big") % (2**31)


def run_init_policy_mlp(*, run_dir: Path, symbol: str, job_id: str) -> dict[str, Any]:
    """Write ``policy/policy_mlp.npz``; returns paths and metadata for job detail."""
    policy_dir = run_dir / "policy"
    policy_dir.mkdir(parents=True, exist_ok=True)
    seed = _policy_seed(symbol, job_id)
    pol = MultiBranchMLPPolicy(seed=seed)
    npz_path = policy_dir / "policy_mlp.npz"
    pol.save(npz_path)
    return {
        "symbol": symbol.strip(),
        "trainer": "init_policy_mlp_materialize",
        "policy_seed": seed,
        "policy_mlp_path": str(npz_path.resolve()),
        "policy_dir": str(policy_dir.resolve()),
    }


def init_policy_detail_payload(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        "symbol": payload.get("symbol"),
        "trainer": payload.get("trainer"),
        "policy_seed": payload.get("policy_seed"),
        "policy_mlp_path": payload.get("policy_mlp_path"),
    }

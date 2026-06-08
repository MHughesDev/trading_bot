"""Immutable run binding for decision records (FB-CAN-077).

Binds ``config_version``, ``logic_version``, ``dataset_id``, and effective ``seed`` with a
tamper-evident hash so live/replay/shadow runs are attributable and comparable (APEX replay
spec §5–7, config management spec §2).
"""

from __future__ import annotations

import hashlib
import json
from typing import Any

from pydantic import BaseModel, Field, field_validator

from app.contracts.replay_events import ReplayRunContract


def _nonempty_str(v: Any, label: str) -> str:
    s = str(v).strip()
    if not s:
        raise ValueError(f"{label} must be non-empty")
    return s


def contract_identity_hash(contract: ReplayRunContract) -> str:
    """SHA-256 of stable replay contract identity (aligned with ``backtesting.replay_provenance``)."""
    rm = contract.replay_mode
    rm_s = str(rm.value if hasattr(rm, "value") else rm)
    payload = {
        "config_version": contract.config_version,
        "dataset_id": contract.dataset_id,
        "execution_model_profile": contract.execution_model_profile,
        "fault_injection_profile": contract.fault_injection_profile,
        "fault_injection_profile_id": contract.fault_injection_profile_id,
        "instrument_scope": sorted(contract.instrument_scope),
        "logic_version": contract.logic_version,
        "replay_mode": rm_s,
        "replay_run_id": contract.replay_run_id,
        "seed": contract.seed,
        "time_range_end": contract.time_range_end,
        "time_range_start": contract.time_range_start,
    }
    raw = json.dumps(payload, sort_keys=True, default=str, separators=(",", ":"))
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def effective_seed_live(
    *,
    config_version: str,
    logic_version: str,
    dataset_id: str,
) -> int:
    """Deterministic 31-bit seed for live/shadow when no replay contract is present."""
    payload = {
        "config_version": config_version,
        "dataset_id": dataset_id,
        "logic_version": logic_version,
        "binding_kind": "live_metadata",
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    digest = hashlib.sha256(raw.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big", signed=False) % (2**31 - 1)


def deterministic_seed_from_dataset_fp(dataset_fp: str, contract: ReplayRunContract) -> int:
    """Same 31-bit derivation as ``backtesting.replay_provenance.deterministic_seed_from_dataset`` (APEX §5 seed)."""
    payload = {
        "contract_identity_hash": contract_identity_hash(contract),
        "dataset_fingerprint": dataset_fp,
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    digest = hashlib.sha256(raw.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big", signed=False) % (2**31 - 1)


def resolve_effective_seed(
    *,
    replay_contract: ReplayRunContract | None,
    replay_dataset_fingerprint: str | None,
    config_version: str,
    logic_version: str,
    live_dataset_id: str,
) -> int:
    """Resolve the seed stamped on :class:`RunBinding` (contract.seed → dataset FP → live metadata)."""
    if replay_contract is not None:
        if replay_contract.seed is not None:
            return int(replay_contract.seed)
        if replay_dataset_fingerprint is not None:
            return deterministic_seed_from_dataset_fp(replay_dataset_fingerprint, replay_contract)
        ds = str(replay_contract.dataset_id).strip() or "default"
        return effective_seed_live(config_version=config_version, logic_version=logic_version, dataset_id=ds)
    return effective_seed_live(
        config_version=config_version,
        logic_version=logic_version,
        dataset_id=live_dataset_id,
    )


def compute_run_binding_hash(
    *,
    config_version: str,
    logic_version: str,
    dataset_id: str,
    seed_effective: int,
    contract_identity_hash: str | None = None,
    replay_run_id: str | None = None,
) -> str:
    """Tamper-evident linkage over version ids + seed (+ optional replay identity)."""
    payload = {
        "config_version": config_version,
        "contract_identity_hash": contract_identity_hash,
        "dataset_id": dataset_id,
        "logic_version": logic_version,
        "replay_run_id": replay_run_id,
        "seed_effective": int(seed_effective),
    }
    raw = json.dumps(payload, sort_keys=True, default=str, separators=(",", ":"))
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


class RunBinding(BaseModel):
    """Immutable identifiers stamped on each decision record (FB-CAN-077)."""

    schema_version: int = Field(default=1, ge=1)
    config_version: str
    logic_version: str
    dataset_id: str
    seed_effective: int
    run_binding_hash: str
    contract_identity_hash: str | None = None
    replay_run_id: str | None = None

    @field_validator("config_version", "logic_version", "dataset_id", mode="before")
    @classmethod
    def _strip_required(cls, v: Any) -> Any:
        if v is None:
            raise ValueError("required")
        s = str(v).strip()
        if not s:
            raise ValueError("must be non-empty")
        return s


def build_run_binding(
    *,
    config_version: str,
    logic_version: str,
    dataset_id: str,
    seed_effective: int,
    replay_contract: ReplayRunContract | None = None,
) -> RunBinding:
    """Build a :class:`RunBinding`; raises ``ValueError`` if any mandatory id is empty."""
    cv = _nonempty_str(config_version, "config_version")
    lv = _nonempty_str(logic_version, "logic_version")
    ds = _nonempty_str(dataset_id, "dataset_id")
    cih: str | None = None
    rrid: str | None = None
    if replay_contract is not None:
        cih = contract_identity_hash(replay_contract)
        rrid = str(replay_contract.replay_run_id).strip() or None
    rbh = compute_run_binding_hash(
        config_version=cv,
        logic_version=lv,
        dataset_id=ds,
        seed_effective=int(seed_effective),
        contract_identity_hash=cih,
        replay_run_id=rrid,
    )
    return RunBinding(
        config_version=cv,
        logic_version=lv,
        dataset_id=ds,
        seed_effective=int(seed_effective),
        run_binding_hash=rbh,
        contract_identity_hash=cih,
        replay_run_id=rrid,
    )


__all__ = [
    "RunBinding",
    "build_run_binding",
    "compute_run_binding_hash",
    "contract_identity_hash",
    "deterministic_seed_from_dataset_fp",
    "effective_seed_live",
    "resolve_effective_seed",
]

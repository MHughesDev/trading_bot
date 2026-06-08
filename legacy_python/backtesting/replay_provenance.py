"""Deterministic seed policy and reproducibility metadata for replay/sim (FB-CAN-068).

``ReplayRunContract.seed`` overrides ``BacktestExecutionParams.rng_seed`` when both are set.
If neither is set, the effective RNG seed is derived from the dataset fingerprint plus stable
contract identity fields so identical bars + contract produce identical stochastic execution.

Output rows may include ``replay_provenance`` with dataset fingerprint and reproducibility hash.
"""

from __future__ import annotations

import hashlib
import io
import json
from typing import Any

import polars as pl

from app.contracts.replay_events import ReplayRunContract
from backtesting.execution_params import BacktestExecutionParams


def _replay_mode_str(contract: ReplayRunContract) -> str:
    rm = contract.replay_mode
    return str(rm.value if hasattr(rm, "value") else rm)


def contract_identity_hash(contract: ReplayRunContract) -> str:
    """Tamper-evident hash of stable run identity fields (not bar contents)."""
    payload = {
        "config_version": contract.config_version,
        "dataset_id": contract.dataset_id,
        "execution_model_profile": contract.execution_model_profile,
        "fault_injection_profile": contract.fault_injection_profile,
        "fault_injection_profile_id": contract.fault_injection_profile_id,
        "instrument_scope": sorted(contract.instrument_scope),
        "logic_version": contract.logic_version,
        "replay_mode": _replay_mode_str(contract),
        "replay_run_id": contract.replay_run_id,
        "seed": contract.seed,
        "time_range_end": contract.time_range_end,
        "time_range_start": contract.time_range_start,
    }
    raw = json.dumps(payload, sort_keys=True, default=str, separators=(",", ":"))
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def deterministic_seed_from_dataset(dataset_fp: str, contract: ReplayRunContract) -> int:
    """Derive a 31-bit positive int seed from dataset hash + contract identity (APEX replay spec §5 seed)."""
    payload = {
        "contract_identity_hash": contract_identity_hash(contract),
        "dataset_fingerprint": dataset_fp,
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    digest = hashlib.sha256(raw.encode("utf-8")).digest()
    # Keep in signed 32-bit range for broad Random/interop compatibility
    return int.from_bytes(digest[:4], "big", signed=False) % (2**31 - 1)


def effective_replay_seed(
    contract: ReplayRunContract,
    *,
    exec_params: BacktestExecutionParams | None,
    dataset_fp: str,
) -> int:
    """Return the RNG seed for simulation noise (contract → execution params → dataset-derived)."""
    eff, _deriv = resolve_replay_seed(contract, exec_params=exec_params, dataset_fp=dataset_fp)
    return eff


def resolve_replay_seed(
    contract: ReplayRunContract,
    *,
    exec_params: BacktestExecutionParams | None,
    dataset_fp: str,
) -> tuple[int, str]:
    """Return (seed, derivation label) for replay provenance and RNG construction."""
    if contract.seed is not None:
        return int(contract.seed), "contract"
    if exec_params is not None and exec_params.rng_seed is not None:
        return int(exec_params.rng_seed), "execution_params"
    return deterministic_seed_from_dataset(dataset_fp, contract), "dataset_fingerprint"


def dataset_fingerprint(bars: pl.DataFrame) -> str:
    """Stable hash of bar frame content (Parquet bytes — same data → same fingerprint)."""
    buf = io.BytesIO()
    bars.write_parquet(buf)
    return hashlib.sha256(buf.getvalue()).hexdigest()


def multi_dataset_fingerprint(bars_by_symbol: dict[str, pl.DataFrame]) -> str:
    """Fingerprint multiple frames in sorted symbol order."""
    h = hashlib.sha256()
    for sym in sorted(bars_by_symbol.keys()):
        h.update(sym.encode("utf-8"))
        h.update(dataset_fingerprint(bars_by_symbol[sym]).encode("ascii"))
    return h.hexdigest()


def replay_output_hash(rows: list[dict[str, Any]]) -> str:
    """SHA-256 of JSON-serialized rows (for comparing full replay outputs)."""
    payload = json.dumps(rows, sort_keys=True, default=str)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def build_replay_provenance(
    *,
    contract: ReplayRunContract,
    effective_seed: int,
    seed_derivation: str,
    dataset_fp: str,
    row_count: int,
    rows_for_hash: list[dict[str, Any]],
) -> dict[str, Any]:
    """Structured provenance block attached to replay outputs."""
    return {
        "schema_version": 1,
        "replay_run_id": contract.replay_run_id,
        "dataset_id": contract.dataset_id,
        "config_version": contract.config_version,
        "logic_version": contract.logic_version,
        "replay_mode": _replay_mode_str(contract),
        "execution_model_profile": contract.execution_model_profile,
        "instrument_scope": list(contract.instrument_scope),
        "contract_identity_hash": contract_identity_hash(contract),
        "seed_effective": effective_seed,
        "seed_derivation": seed_derivation,
        "dataset_fingerprint": dataset_fp,
        "bar_row_count": row_count,
        "reproducibility_hash": replay_output_hash(rows_for_hash),
    }


def attach_provenance_to_rows(rows: list[dict[str, Any]], provenance: dict[str, Any]) -> None:
    """Mutate rows in place with shared provenance (same dict reference)."""
    for r in rows:
        r["replay_provenance"] = provenance

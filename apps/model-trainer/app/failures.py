"""How a trainer failure is reported to the ledger (SPEC §9, ADR-P2-30).

The platform records every trial's ending with a `TerminalReason`, and the
censoring that follows from it (INV-17) is decided by that reason alone. Before
this module, the Rust side recovered the reason by substring-matching the
exception message the sidecar returned — `contains("nan")`, `contains("data")` —
so a reworded message silently changed a trial's censoring and anything
unrecognised became `dependency_failure`.

The classification happens here instead, on the **type** of the exception, in
the process that actually knows what went wrong.

What this module does *not* do is pretend to be total. An exception type nobody
has classified maps to `dependency_failure`, and that is a true statement — the
job depended on something that raised and did not say what — but it is not the
same as knowing. `train_torch_model` raising `NanDivergence` on a non-finite
loss is what turns `nan_divergence` from a label into an observation; every
other framework that can diverge should grow the same guard.
"""

from __future__ import annotations

import asyncio

# The closed set from SPEC §9. These strings are the wire contract with
# `ledger::TerminalReason::as_str`; changing one here changes a ledger row.
OOM = "oom"
NAN_DIVERGENCE = "nan_divergence"
DATA_ERROR = "data_error"
TIMEOUT = "timeout"
LEAKAGE_DETECTED = "leakage_detected"
BUDGET_EXCEEDED = "budget_exceeded"
CANCELLED = "cancelled"
DEPENDENCY_FAILURE = "dependency_failure"


class TrainerFailure(Exception):
    """A failure that names its own terminal reason."""

    terminal = DEPENDENCY_FAILURE


class NanDivergence(TrainerFailure):
    """Training produced a non-finite loss or weight."""

    terminal = NAN_DIVERGENCE


class DataError(TrainerFailure):
    """The dataset is absent, empty, or unusable for this definition."""

    terminal = DATA_ERROR


class BudgetExceeded(TrainerFailure):
    """The run hit its `max_gpu_hours` and was stopped (SPEC §12.5)."""

    terminal = BUDGET_EXCEEDED


def _is_cuda_oom(exc: BaseException) -> bool:
    """`torch.cuda.OutOfMemoryError` without importing torch to find out."""
    return type(exc).__name__ in ("OutOfMemoryError", "CudaOutOfMemoryError")


def classify(exc: BaseException) -> str:
    """The terminal reason for `exc`.

    Matched on type, never on message text: an exception's class is a decision
    the raising code made, its message is prose that gets reworded.
    """
    if isinstance(exc, TrainerFailure):
        return exc.terminal
    if isinstance(exc, MemoryError) or _is_cuda_oom(exc):
        return OOM
    if isinstance(exc, FloatingPointError):
        return NAN_DIVERGENCE
    if isinstance(exc, (asyncio.TimeoutError, TimeoutError)):
        return TIMEOUT
    if isinstance(exc, asyncio.CancelledError):
        return CANCELLED
    if isinstance(exc, (FileNotFoundError, KeyError)):
        # A column the definition names that the frame does not have, or an
        # artifact that is not where the manifest says. Both are the data plane
        # failing to supply what was asked for.
        return DATA_ERROR
    return DEPENDENCY_FAILURE

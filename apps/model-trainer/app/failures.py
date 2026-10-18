"""How a trainer failure is reported to the ledger (SPEC §9, ADR-P2-30).

The platform records every trial's ending with a `TerminalReason`, and the
censoring that follows from it (INV-17) is decided by that reason alone. Before
this module, the Rust side recovered the reason by substring-matching the
exception message the sidecar returned — `contains("nan")`, `contains("data")` —
so a reworded message silently changed a trial's censoring and anything
unrecognised became `dependency_failure`.

The classification happens here instead, on the **type** of the exception, in
the process that actually knows what went wrong.

The classification is **total** over everything Python raises by itself (every
`builtins` exception, plus the asyncio and concurrent.futures ones), and says
which kind of answer it gave: [`Classification.basis`] is `declared` (the
exception named its own reason), `type_table` (a decision recorded in
`_TYPE_TABLE`) or `unclassified` (a third-party type no entry reaches). The last
still maps to `dependency_failure`, because the ledger's reason set is closed
(SPEC 9) - but it is now a *visible* fallback, carried to the platform with the
exception's type name, instead of being indistinguishable from a decision.
`tests/test_failures.py` enumerates the builtins and fails if one lacks an
entry. `train_torch_model` raising `NanDivergence` on a non-finite loss is what
turns `nan_divergence` from a label into an observation; every other framework
that can diverge should grow the same guard.
"""

from __future__ import annotations

import asyncio
import builtins as _builtins
import concurrent.futures as _futures
from dataclasses import dataclass

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


# ---------------------------------------------------------------------------
# The classification table
#
# Bare `Exception` and `BaseException` are deliberately NOT entries. A root entry
# would answer every unlisted class, and "I do not know" would then be
# indistinguishable from "I decided".
# ---------------------------------------------------------------------------

_TYPE_TABLE: dict[type, str] = {}


def _declare(reason: str, *classes: type) -> None:
    for cls in classes:
        _TYPE_TABLE[cls] = reason


# Resource exhaustion.
_declare(OOM, MemoryError)

# Arithmetic that went non-finite or out of range: the numeric signature of a
# diverging fit. `FloatingPointError` is what `np.seterr(all="raise")` raises.
_declare(NAN_DIVERGENCE, ArithmeticError)

# Ran out of time, or was told to stop.
_declare(TIMEOUT, TimeoutError, _futures.TimeoutError, asyncio.TimeoutError)
_declare(
    CANCELLED,
    asyncio.CancelledError,
    _futures.CancelledError,
    KeyboardInterrupt,
    SystemExit,
    GeneratorExit,
)

# The data plane failing to supply what was asked for: a column the definition
# names that the frame lacks, an artifact not where the manifest says, a value
# the data supplied that the estimator cannot accept, an undecodable stream.
_declare(
    DATA_ERROR,
    LookupError,  # KeyError, IndexError
    FileNotFoundError,
    IsADirectoryError,
    NotADirectoryError,
    ValueError,  # includes UnicodeError and numpy's LinAlgError
    EOFError,
    BufferError,
)

# Everything else the interpreter raises: the job depended on code, a library, a
# socket or a file system that raised and the process cannot say more. Declared
# rather than defaulted - these are decisions about *known* types.
_declare(
    DEPENDENCY_FAILURE,
    OSError,  # connection, permission, broken pipe... (narrower entries win)
    ImportError,
    AttributeError,
    NameError,
    TypeError,
    AssertionError,
    RuntimeError,  # includes NotImplementedError and RecursionError
    SyntaxError,
    SystemError,
    ReferenceError,
    StopIteration,
    StopAsyncIteration,
    Warning,  # a warning promoted to an error
    _futures.BrokenExecutor,
)
# A group is resolved structurally by `classify_detailed` (its members' common
# reason); this entry is only what an *empty* or disagreeing group falls back to.
if hasattr(_builtins, "BaseExceptionGroup"):  # Python 3.11+
    _declare(DEPENDENCY_FAILURE, _builtins.BaseExceptionGroup)


@dataclass(frozen=True)
class Classification:
    """A terminal reason and how it was arrived at."""

    reason: str
    # "declared"     the exception carries its own `terminal`
    # "type_table"   resolved through `_TYPE_TABLE`
    # "unclassified" nothing reached it; `reason` is the closed set's fallback
    basis: str
    exception_type: str


def _table_reason(cls: type) -> str | None:
    """The reason for the nearest class in the MRO that has an entry."""
    for base in cls.__mro__:
        reason = _TYPE_TABLE.get(base)
        if reason is not None:
            return reason
    return None


def builtin_exception_types() -> list[type]:
    """Every exception class in `builtins`. The totality test's universe."""
    return [
        v
        for v in vars(_builtins).values()
        if isinstance(v, type) and issubclass(v, BaseException)
    ]


def classify_detailed(exc: BaseException) -> Classification:
    """The terminal reason for `exc`, and whether anyone had classified it.

    Matched on type, never on message text: an exception's class is a decision
    the raising code made, its message is prose that gets reworded.
    """
    name = f"{type(exc).__module__}.{type(exc).__qualname__}"
    if isinstance(exc, TrainerFailure):
        return Classification(exc.terminal, "declared", name)
    if _is_cuda_oom(exc):
        return Classification(OOM, "type_table", name)
    # An exception group is as bad as its members: if they agree, that is the
    # reason; if they do not, the group cannot be summarised by any one of them.
    group = getattr(_builtins, "BaseExceptionGroup", None)  # Python 3.11+
    members = getattr(exc, "exceptions", None)
    if group is not None and isinstance(exc, group) and members:
        reasons = {classify_detailed(m).reason for m in members}
        if len(reasons) == 1:
            return Classification(reasons.pop(), "type_table", name)
        return Classification(DEPENDENCY_FAILURE, "type_table", name)
    reason = _table_reason(type(exc))
    if reason is not None:
        return Classification(reason, "type_table", name)
    return Classification(DEPENDENCY_FAILURE, "unclassified", name)


def classify(exc: BaseException) -> str:
    """The terminal reason for `exc` (see [`classify_detailed`])."""
    return classify_detailed(exc).reason

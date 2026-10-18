"""The trainer's failure classification is total (HANDOFF 4; SPEC 9, ADR-P2-30).

The property: every exception Python can raise on its own resolves to a declared
reason from the closed set, and the only way to reach `unclassified` is to be a
type nothing in the table reaches. A new Python release that adds a builtin
exception fails the first test rather than falling through silently.
"""

from __future__ import annotations

import asyncio
import concurrent.futures

import pytest

from app import failures

CLOSED_SET = {
    failures.OOM,
    failures.NAN_DIVERGENCE,
    failures.DATA_ERROR,
    failures.TIMEOUT,
    failures.LEAKAGE_DETECTED,
    failures.BUDGET_EXCEEDED,
    failures.CANCELLED,
    failures.DEPENDENCY_FAILURE,
}


def test_every_builtin_exception_resolves_to_a_declared_reason():
    unreached = []
    for cls in failures.builtin_exception_types():
        if cls in (BaseException, Exception):
            # The roots are deliberately not table entries; see below.
            continue
        reason = failures._table_reason(cls)
        if reason is None:
            unreached.append(cls.__name__)
        else:
            assert reason in CLOSED_SET, (cls, reason)
    assert not unreached, f"builtin exceptions with no classification: {unreached}"


def test_the_roots_are_not_entries_so_unknown_is_distinguishable_from_decided():
    assert BaseException not in failures._TYPE_TABLE
    assert Exception not in failures._TYPE_TABLE


class ThirdPartyError(Exception):
    """Stands in for a library's own exception type."""


def test_an_unknown_third_party_type_is_a_visible_fallback():
    c = failures.classify_detailed(ThirdPartyError("boom"))
    assert c.reason == failures.DEPENDENCY_FAILURE
    assert c.basis == "unclassified"
    assert c.exception_type.endswith("ThirdPartyError")


def test_a_subclass_inherits_its_nearest_declared_ancestor():
    class MyKeyError(KeyError):
        pass

    c = failures.classify_detailed(MyKeyError("col"))
    assert (c.reason, c.basis) == (failures.DATA_ERROR, "type_table")


@pytest.mark.parametrize(
    "exc, reason",
    [
        (MemoryError(), failures.OOM),
        (FloatingPointError(), failures.NAN_DIVERGENCE),
        (ZeroDivisionError(), failures.NAN_DIVERGENCE),
        (TimeoutError(), failures.TIMEOUT),
        (asyncio.TimeoutError(), failures.TIMEOUT),
        (concurrent.futures.TimeoutError(), failures.TIMEOUT),
        (asyncio.CancelledError(), failures.CANCELLED),
        (KeyboardInterrupt(), failures.CANCELLED),
        (FileNotFoundError(), failures.DATA_ERROR),
        (KeyError("c"), failures.DATA_ERROR),
        (ValueError("bad"), failures.DATA_ERROR),
        (ConnectionResetError(), failures.DEPENDENCY_FAILURE),
        (ImportError(), failures.DEPENDENCY_FAILURE),
        (TypeError(), failures.DEPENDENCY_FAILURE),
        (failures.NanDivergence("nan loss"), failures.NAN_DIVERGENCE),
        (failures.DataError("empty"), failures.DATA_ERROR),
        (failures.BudgetExceeded("gpu"), failures.BUDGET_EXCEEDED),
    ],
)
def test_known_types_keep_their_reasons(exc, reason):
    assert failures.classify(exc) == reason


def test_a_declared_failure_says_so():
    c = failures.classify_detailed(failures.NanDivergence("nan"))
    assert c.basis == "declared"


def test_cuda_oom_is_recognised_without_importing_torch():
    class OutOfMemoryError(RuntimeError):
        pass

    c = failures.classify_detailed(OutOfMemoryError("CUDA out of memory"))
    assert (c.reason, c.basis) == (failures.OOM, "type_table")


@pytest.mark.skipif(not hasattr(__builtins__, "ExceptionGroup") and not hasattr(__import__("builtins"), "ExceptionGroup"), reason="needs Python 3.11")
def test_an_exception_group_is_its_members_when_they_agree():
    g = ExceptionGroup("many", [KeyError("a"), FileNotFoundError("b")])  # noqa: F821
    assert failures.classify(g) == failures.DATA_ERROR
    mixed = ExceptionGroup("mixed", [KeyError("a"), MemoryError()])  # noqa: F821
    assert failures.classify(mixed) == failures.DEPENDENCY_FAILURE


def test_every_reason_the_table_can_produce_is_in_the_closed_set():
    assert set(failures._TYPE_TABLE.values()) <= CLOSED_SET

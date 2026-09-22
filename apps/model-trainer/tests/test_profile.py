"""The population half of the `InstrumentProfile` job (checklist 3.1/3.4)."""

from __future__ import annotations

import numpy as np
import pytest

from app import embedding, profile
from app.fingerprint import Bars, compute


def synthetic_bars(n: int = 400, seed: int = 0, scale: float = 0.01) -> Bars:
    rng = np.random.default_rng(seed)
    steps = rng.normal(0.0, scale, n)
    close = 100.0 * np.exp(np.cumsum(steps))
    high = close * (1.0 + np.abs(rng.normal(0.0, scale / 2, n)))
    low = close * (1.0 - np.abs(rng.normal(0.0, scale / 2, n)))
    open_ = np.concatenate([[100.0], close[:-1]])
    ts = np.arange(n, dtype=np.int64) * 3_600_000_000_000
    return Bars(
        ts_ns=ts,
        open=open_,
        high=np.maximum(high, np.maximum(open_, close)),
        low=np.minimum(low, np.minimum(open_, close)),
        close=close,
        volume=rng.uniform(1_000.0, 5_000.0, n),
    )


def population(n_instruments: int, bars_len: int = 400) -> list[tuple[int, int, Bars]]:
    return [
        (1000 + i, 4, synthetic_bars(bars_len, seed=i, scale=0.005 + 0.0002 * i))
        for i in range(n_instruments)
    ]


def test_the_declared_order_covers_exactly_what_compute_produces():
    """`tier1` is a bare REAL[], so position is meaning.

    Deriving the order from `sorted(fp.values)` works until an instrument is
    missing a dimension, at which point every slot after it shifts and two rows
    written a day apart stop describing the same thing. This is the test that
    keeps the declared order and the computed one from drifting apart — a new
    dimension added to `compute` fails here rather than silently never being
    stored.
    """
    fp = compute(synthetic_bars())
    produced = set(fp.values) | set(fp.absent)
    declared = set(profile.TIER1_DIMENSIONS)
    assert produced <= declared, f"not stored anywhere: {sorted(produced - declared)}"
    assert declared <= produced, f"declared but never computed: {sorted(declared - produced)}"
    assert len(profile.TIER1_DIMENSIONS) == len(set(profile.TIER1_DIMENSIONS))


def test_a_small_population_is_not_quietly_whitened():
    """Below sixty instruments `fit_whitener` refuses, and the batch says so.

    The failure this prevents is the silent one: a projection that still returns
    48 numbers, computed a different way, under the same name. Every row carries
    the flag so the caller cannot store it as though it came from the fitted
    transform.
    """
    batch = profile.profile_population(population(5))
    assert batch.whitened is False
    assert batch.reason and "60" in batch.reason
    assert batch.instruments
    for row in batch.instruments:
        assert row.quality_flags & profile.QUALITY_UNWHITENED


def test_a_large_enough_population_is_whitened_and_flagged_clean():
    batch = profile.profile_population(population(embedding.MIN_POPULATION))
    assert batch.whitened is True
    assert batch.reason is None
    for row in batch.instruments:
        assert not row.quality_flags & profile.QUALITY_UNWHITENED


def test_every_retrieval_vector_is_the_column_width_and_unit_length():
    """`vector(48)` is a fixed width and the metric is cosine.

    Padding is zeros rather than anything else because a dimension that is zero
    for every row contributes nothing to any distance — it cannot move a
    neighbour list, which is what makes it safe filler here and unsafe in
    `tier1`.
    """
    batch = profile.profile_population(population(8))
    for row in batch.instruments:
        assert len(row.retrieval_vec) == embedding.RETRIEVAL_DIMS
        assert np.isclose(np.linalg.norm(row.retrieval_vec), 1.0, atol=1e-9)
        assert all(np.isfinite(row.retrieval_vec))
        assert all(v == 0.0 for v in row.retrieval_vec[len(profile.TIER1_DIMENSIONS):])


def test_an_absent_dimension_stays_absent_in_tier1():
    """Imputation is for the projection only.

    "We do not know this instrument's overnight share" and "we placed it at the
    population median to take a distance" are different claims, and a consumer
    reading `tier1` must see the first. A `0.0` here is a measurement nobody
    made, and it ends up inside a kNN distance.
    """
    # Twenty bars: long enough to measure the short horizons, far too short for
    # rv_252 and Hurst.
    short = [(2000, 4, synthetic_bars(20, seed=7))]
    batch = profile.profile_population(short + population(4))
    row = next(r for r in batch.instruments if r.instrument_id == 2000)
    assert any(v is None for v in row.tier1)
    assert row.quality_flags & profile.QUALITY_IMPUTED_DIMENSION
    assert row.absent, "the reasons travel with the row"
    # The projection still produced a finite, usable vector.
    assert all(np.isfinite(row.retrieval_vec))


def test_an_instrument_that_measured_nothing_is_dropped_not_written_as_nulls():
    """A row that says nothing is not knowledge, and it would still occupy a
    cell in every neighbour search."""
    flat = Bars(
        ts_ns=np.array([0, 1], dtype=np.int64),
        open=np.array([1.0, 1.0]),
        high=np.array([1.0, 1.0]),
        low=np.array([1.0, 1.0]),
        close=np.array([1.0, 1.0]),
        volume=np.array([0.0, 0.0]),
    )
    batch = profile.profile_population([(3000, 4, flat)])
    assert batch.instruments == []
    assert batch.reason


def test_the_same_population_profiles_identically_twice():
    """The job is a pure function of its manifest: re-running one after a lost
    lease has to produce the row that is already there, or the upsert that makes
    scheduling idempotent would rewrite a different vector each pass."""
    first = profile.profile_population(population(6))
    second = profile.profile_population(population(6))
    assert first.as_dict() == second.as_dict()


def test_the_intrinsic_dimension_is_absent_rather_than_invented():
    """Below ten rows the two-nearest-neighbour estimator has nothing to
    estimate from, and a number invented there would be monitored as though it
    meant something."""
    assert profile.profile_population(population(4)).intrinsic_dim is None
    assert profile.profile_population(population(20)).intrinsic_dim is not None


@pytest.mark.parametrize("n", [1, 2, 3])
def test_a_population_of_one_or_two_still_produces_a_finite_vector(n: int):
    batch = profile.profile_population(population(n))
    assert len(batch.instruments) == n
    for row in batch.instruments:
        assert all(np.isfinite(row.retrieval_vec))

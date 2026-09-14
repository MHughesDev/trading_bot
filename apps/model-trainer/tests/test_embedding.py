"""Whitening, hubness and the embedding validation protocol (3.4/3.11)."""
from __future__ import annotations
import pathlib, sys
import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import embedding as emb  # noqa: E402


def population(n=200, features=60, seed=1):
    r = np.random.default_rng(seed)
    # Scales spanning nine orders of magnitude, like the real fingerprint block.
    scales = np.logspace(-9, 6, features)
    return r.normal(0, 1, (n, features)) * scales


def test_whitening_refuses_a_population_too_small_to_estimate_from():
    with pytest.raises(emb.NotEnoughPopulation):
        emb.fit_whitener(population(n=20))


def test_whitening_puts_wildly_different_scales_on_one_footing():
    """A Euclidean distance over raw fingerprints is a distance in turnover with
    rounding error attached."""
    pop = population()
    w = emb.fit_whitener(pop)
    out = w.transform(pop)
    assert out.shape[1] == emb.RETRIEVAL_DIMS
    # Every retrieval vector is unit length: the metric is cosine.
    assert np.allclose(np.linalg.norm(out, axis=1), 1.0, atol=1e-9)
    # No single component dominates the way the raw block's largest scale does.
    raw_share = pop.var(axis=0).max() / pop.var(axis=0).sum()
    white_share = out.var(axis=0).max() / out.var(axis=0).sum()
    assert raw_share > 0.6, "the fixture really is dominated by one scale"
    assert white_share < raw_share / 2, (
        f"whitening should spread the variance: {white_share} vs {raw_share}"
    )


def test_mutual_proximity_demotes_a_hub():
    """A hub is close to everyone from its own side; looking from one end alone
    cannot tell that apart from genuine similarity."""
    n = 40
    r = np.random.default_rng(3)
    pts = r.normal(0, 1, (n, 5))
    pts[0] = 0.0  # a point at the centre of everything: the hub
    d = np.sqrt(((pts[:, None, :] - pts[None, :, :]) ** 2).sum(axis=2))
    mp = emb.mutual_proximity(d)
    # How often the hub is someone's nearest neighbour, before and after.
    def hub_wins(matrix):
        m = matrix.copy()
        np.fill_diagonal(m, np.inf)
        return int((np.argmin(m, axis=1) == 0).sum())
    assert hub_wins(mp) <= hub_wins(d)
    assert np.allclose(np.diag(mp), 0.0)


def test_intrinsic_dimension_notices_a_low_dimensional_manifold():
    """45 noise directions in a 48-d embedding is the hubness coming back."""
    r = np.random.default_rng(5)
    t = r.uniform(0, 1, (300, 2))
    embedded = np.hstack([t, np.zeros((300, 10))])
    d = emb.intrinsic_dimension(embedded)
    assert d is not None and d < 4, d
    full = emb.intrinsic_dimension(r.normal(0, 1, (300, 8)))
    assert full > d


def test_an_unmeasured_criterion_fails_validation():
    """\"We could not check\" is not \"it is fine\": a version that becomes the
    default unchecked silently changes every neighbour list."""
    ok, why = emb.ValidationReport(0.9, 2.0, None).passes()
    assert not ok and "temporal ARI was not measured" in why

    ok, why = emb.ValidationReport(0.9, 2.0, 0.8).passes()
    assert ok, why


def test_validation_names_every_failing_criterion():
    ok, why = emb.ValidationReport(0.1, 1.0, 0.1).passes()
    assert not ok
    for name in ("precision@k", "transfer lift", "temporal ARI"):
        assert name in why


def test_precision_at_k_counts_shared_labels():
    truth = {"a": "x", "b": "x", "c": "y"}
    # Rows line up with truth's keys: a→b (same label), b→a (same), c→a (not).
    assert emb.precision_at_k([["b", "c"], ["a", "c"], ["a", "b"]], truth, 1) == pytest.approx(2 / 3)
    # Everyone's nearest is c, which only matches for c itself — and c's is a.
    assert emb.precision_at_k([["c"], ["c"], ["a"]], truth, 1) == pytest.approx(0.0)


def test_transfer_lift_is_measured_against_a_random_baseline():
    """An absolute transfer number says how easy the tasks were; the ratio says
    whether the embedding contributed."""
    assert emb.transfer_lift([2.0] * 10, [1.0] * 10) == pytest.approx(2.0)
    assert emb.transfer_lift([1.0] * 3, [1.0] * 10) is None, "too few observations"
    assert emb.transfer_lift([1.0] * 10, [0.0] * 10) is None, "a zero baseline is no baseline"


def test_the_adjusted_rand_index_is_one_for_a_relabelling():
    a = [0, 0, 1, 1, 2, 2]
    assert emb.adjusted_rand_index(a, [5, 5, 9, 9, 7, 7]) == pytest.approx(1.0)
    assert emb.adjusted_rand_index(a, [0, 1, 2, 0, 1, 2]) < 0.3

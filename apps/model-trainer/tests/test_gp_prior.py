"""The √D lengthscale prior (SPEC 11.1, checklist 2.7)."""

from __future__ import annotations

import math
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

from app import gp_prior  # noqa: E402


def test_the_median_lengthscale_scales_as_the_root_of_the_dimension():
    """The whole point: typical pairwise distance in D normalised dimensions
    grows like √D, and the prior's median has to follow it or the GP concludes
    every point is independent of every other."""
    for dims in (1, 4, 16, 64):
        p = gp_prior.lengthscale_prior(dims)
        assert p["median"] == pytest.approx(math.sqrt(dims))

    # Quadrupling the dimension doubles the median.
    assert gp_prior.lengthscale_prior(16)["median"] == pytest.approx(
        2 * gp_prior.lengthscale_prior(4)["median"]
    )


def test_one_dimension_is_the_unit_cube_baseline():
    p = gp_prior.lengthscale_prior(1)
    assert p["log_mean"] == pytest.approx(0.0)
    assert p["median"] == pytest.approx(1.0)


def test_the_prior_is_lognormal_with_the_declared_spread():
    p = gp_prior.lengthscale_prior(9, sigma0=0.5)
    assert p["distribution"] == "lognormal"
    assert p["log_std"] == pytest.approx(0.5)
    assert p["log_mean"] == pytest.approx(math.log(9) / 2)


def test_a_degenerate_space_is_a_caller_mistake_not_a_handled_case():
    for dims in (0, -3):
        with pytest.raises(ValueError, match="at least one dimension"):
            gp_prior.lengthscale_prior(dims)
    with pytest.raises(ValueError, match="positive spread"):
        gp_prior.lengthscale_prior(4, sigma0=0.0)

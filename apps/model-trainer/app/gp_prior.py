"""The √D-scaled lengthscale prior for GP-BO (SPEC 11.1, checklist 2.7).

    l ~ LogNormal(mu0 + log(D)/2, sigma0)

One line of arithmetic, and the reason it matters is geometric rather than
statistical. In D dimensions, points drawn from any reasonable distribution are
farther apart than in one dimension — typical pairwise distance grows like √D.
A GP whose lengthscale prior does not know that puts nearly all its prior mass
on lengthscales far shorter than any real distance in the space, concludes that
every observation is independent of every other, and returns the prior mean
everywhere. The surrogate is then a very expensive random search.

Scaling the *median* of the LogNormal by √D — which is what adding log(D)/2 to
the log-mean does — puts the prior where the distances actually are.

Deliberately not here: trust regions and learned embeddings. Both are second
moves, both add parameters, and neither is worth anything until the basic prior
is right (ADR-P2-08 defers GP-BO for strategy parameters entirely).
"""

from __future__ import annotations

import math

# The one-dimensional baseline, on inputs normalised to the unit cube. A median
# lengthscale of e^0 = 1 is the width of the cube: the prior starts out saying
# "the function might vary over the whole space" and lets the data shorten it.
MU0 = 0.0

# Prior spread in log-space. Wide enough to reach an order of magnitude either
# side of the median within one standard deviation, which covers the range
# between "smooth across the cube" and "varies within a tenth of it".
SIGMA0 = 1.0


def lengthscale_log_mean(dims: int, mu0: float = MU0) -> float:
    """`mu0 + log(D)/2` — the log-mean of the lengthscale prior.

    # Raises
    `ValueError` for a non-positive dimensionality: a GP over zero dimensions is
    not a degenerate case to handle, it is a caller mistake to report.
    """
    if dims <= 0:
        raise ValueError(f"a GP needs at least one dimension, got {dims}")
    return mu0 + math.log(dims) / 2.0


def lengthscale_prior(dims: int, mu0: float = MU0, sigma0: float = SIGMA0) -> dict:
    """The prior as `{distribution, log_mean, log_std, median}`.

    `median` is `exp(log_mean)`, which is the quantity worth eyeballing: it
    should be close to the typical distance between points in the space, and in
    D normalised dimensions that distance is about √D.
    """
    if sigma0 <= 0:
        raise ValueError("the lengthscale prior needs positive spread")
    log_mean = lengthscale_log_mean(dims, mu0)
    return {
        "distribution": "lognormal",
        "log_mean": log_mean,
        "log_std": sigma0,
        "median": math.exp(log_mean),
    }

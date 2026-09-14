"""Whitening, hubness correction and embedding validation
(SPEC 5.3, checklist 3.4/3.11, ADR-P3-05).

Three steps between a raw fingerprint and something you can take a nearest
neighbour in, and each exists because skipping it produces a neighbour list that
is confidently wrong in a specific way.

**Whitening.** The tier-1 block mixes quantities on wildly different scales — a
Hurst exponent near 0.5, an Amihud illiquidity near 1e-9, a turnover in the
millions. A Euclidean distance over that is a distance in turnover with rounding
error attached. PCA whitening puts every direction on the same footing, and
`retrieval_vec` is the first 48 components because that is the column width the
schema fixes.

**Hubness correction.** In high dimension a few points become everyone's nearest
neighbour — "hubs" — for reasons that are a property of the space rather than of
the data. Mutual proximity rescales each distance by how surprising it is from
*both* ends, which is what makes "BTC is similar to everything" stop being the
answer.

**Validation.** A new `embedding_version` becomes the default only by passing
(3.11): retrieval precision against held-out labels, transfer lift against a
random baseline, and temporal stability between versions. An embedding nobody
validated is a similarity metric nobody designed.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

# The width `knowledge.asset_embedding.retrieval_vec` fixes.
RETRIEVAL_DIMS = 48

# Below this many instruments, whitening estimates a covariance from fewer
# points than it has dimensions and produces directions that are pure noise.
MIN_POPULATION = 60


class NotEnoughPopulation(ValueError):
    """Too few instruments to whiten against."""


@dataclass
class Whitener:
    """A fitted PCA whitening transform, per `embedding_version`.

    Fitted over the **market-public population** — every instrument, not the
    ones a caller happens to be asking about. A transform fitted on the query
    set would make the geometry depend on the question.
    """

    mean: np.ndarray
    components: np.ndarray  # (dims, features)
    scale: np.ndarray  # per-component standard deviation

    def transform(self, x: np.ndarray) -> np.ndarray:
        """Project and whiten, then L2-normalise.

        Normalised because the retrieval metric is cosine: without it, an
        instrument that is simply *more extreme* in every direction ends up
        close to nothing, which is a magnitude artefact rather than a
        similarity one.
        """
        centred = np.atleast_2d(np.asarray(x, dtype=np.float64)) - self.mean
        projected = centred @ self.components.T / np.clip(self.scale, 1e-12, None)
        norms = np.linalg.norm(projected, axis=1, keepdims=True)
        return projected / np.clip(norms, 1e-12, None)


def fit_whitener(population: np.ndarray, dims: int = RETRIEVAL_DIMS) -> Whitener:
    """Fit PCA whitening over the population.

    # Raises
    `NotEnoughPopulation` below `MIN_POPULATION` rows. Refusing is the point: a
    covariance estimated from fewer points than dimensions yields components
    that are noise, and a kNN over them returns confident nonsense.
    """
    x = np.atleast_2d(np.asarray(population, dtype=np.float64))
    if x.shape[0] < MIN_POPULATION:
        raise NotEnoughPopulation(
            f"whitening needs at least {MIN_POPULATION} instruments, got {x.shape[0]}"
        )
    mean = x.mean(axis=0)
    centred = x - mean
    # SVD rather than an explicit covariance: better conditioned, and it does not
    # square the dynamic range of a block whose scales already span nine orders
    # of magnitude.
    _, s, vt = np.linalg.svd(centred, full_matrices=False)
    k = min(dims, vt.shape[0])
    scale = s[:k] / np.sqrt(max(x.shape[0] - 1, 1))
    return Whitener(mean=mean, components=vt[:k], scale=scale)


def mutual_proximity(distances: np.ndarray) -> np.ndarray:
    """Rescale a distance matrix by mutual proximity (Schnitzer et al.).

    `MP(i, j)` is the probability that `j` is farther from `i` than a random
    point *and* `i` is farther from `j` than a random point. Both ends matter:
    a hub is close to everyone from its own side, and only looking from the
    query's side cannot tell that apart from genuine similarity.

    Returns a matrix where **smaller is still closer**, so it drops into a kNN
    unchanged.
    """
    d = np.asarray(distances, dtype=np.float64)
    n = d.shape[0]
    if n < 3:
        return d
    mu = d.mean(axis=1, keepdims=True)
    sd = np.clip(d.std(axis=1, ddof=1, keepdims=True), 1e-12, None)
    # P(distance from i to a random point > d_ij), under a normal approximation.
    from math import erf, sqrt

    def sf(z: np.ndarray) -> np.ndarray:
        return 0.5 * (1.0 - np.vectorize(lambda v: erf(v / sqrt(2.0)))(z))

    z = (d - mu) / sd
    p_from_i = sf(z)
    p_from_j = sf(((d - mu.T) / sd.T))
    mp = p_from_i * p_from_j
    # Back to a distance: 1 − MP, so smaller stays closer.
    out = 1.0 - mp
    np.fill_diagonal(out, 0.0)
    return out


def intrinsic_dimension(vectors: np.ndarray) -> float | None:
    """Two-nearest-neighbour intrinsic dimension (Facco et al.).

    Monitored because it is the number that says whether the 48 dimensions are
    doing anything. An intrinsic dimension of 3 in a 48-d embedding means 45
    directions are noise, and the hubness the whitening was meant to fix is
    about to come back.
    """
    x = np.atleast_2d(np.asarray(vectors, dtype=np.float64))
    n = x.shape[0]
    if n < 10:
        return None
    diff = x[:, None, :] - x[None, :, :]
    d = np.sqrt((diff**2).sum(axis=2))
    np.fill_diagonal(d, np.inf)
    nearest = np.sort(d, axis=1)[:, :2]
    r1, r2 = nearest[:, 0], nearest[:, 1]
    ok = (r1 > 0) & np.isfinite(r2)
    if ok.sum() < 10:
        return None
    mu = r2[ok] / r1[ok]
    # The maximum-likelihood estimate: d = N / sum(log mu).
    denom = float(np.log(mu).sum())
    return float(ok.sum() / denom) if denom > 0 else None


# ─────────────────────────────────────────────────────────────────────────────
# 3.11 — the validation protocol
# ─────────────────────────────────────────────────────────────────────────────

# A new version has to clear all three. Any one of them failing means the
# embedding is worse than what it would replace in a way that matters.
MIN_PRECISION_AT_K = 0.5
MIN_TRANSFER_LIFT = 1.2
MIN_TEMPORAL_ARI = 0.5


@dataclass
class ValidationReport:
    precision_at_k: float | None
    transfer_lift: float | None
    temporal_ari: float | None

    def passes(self) -> tuple[bool, str]:
        """Whether this version may become the default.

        An **unmeasured** criterion fails. "We could not check" is not "it is
        fine", and an embedding version that becomes the default without being
        checked silently changes every neighbour list in the platform.
        """
        checks = [
            ("precision@k", self.precision_at_k, MIN_PRECISION_AT_K),
            ("transfer lift", self.transfer_lift, MIN_TRANSFER_LIFT),
            ("temporal ARI", self.temporal_ari, MIN_TEMPORAL_ARI),
        ]
        failures = []
        for name, value, floor in checks:
            if value is None:
                failures.append(f"{name} was not measured")
            elif value < floor:
                failures.append(f"{name} {value:.3f} < {floor}")
        if failures:
            return False, "; ".join(failures)
        return True, (
            f"precision@k {self.precision_at_k:.3f}, transfer lift "
            f"{self.transfer_lift:.3f}, temporal ARI {self.temporal_ari:.3f}"
        )


def precision_at_k(neighbours: list[list[str]], truth: dict[str, str], k: int) -> float | None:
    """Fraction of the top-k that share the query's held-out cluster label."""
    if not neighbours or k <= 0:
        return None
    hits = total = 0
    for query, ranked in zip(truth, neighbours):
        label = truth.get(query)
        if label is None:
            continue
        for other in ranked[:k]:
            total += 1
            if truth.get(other) == label:
                hits += 1
    return hits / total if total else None


def transfer_lift(with_embedding: list[float], random_baseline: list[float]) -> float | None:
    """How much better transfer is with the embedding than with a random pick.

    Against a **random baseline**, not against nothing: an absolute transfer
    number says how easy the tasks were, and the ratio says whether the
    embedding contributed.
    """
    a = [v for v in with_embedding if np.isfinite(v)]
    b = [v for v in random_baseline if np.isfinite(v)]
    if len(a) < 5 or len(b) < 5:
        return None
    base = float(np.mean(b))
    if base <= 0:
        return None
    return float(np.mean(a) / base)


def adjusted_rand_index(a: list[int], b: list[int]) -> float | None:
    """ARI between two clusterings of the same points — 3.11's temporal
    stability. A version that reshuffles the clusters every refit is not a more
    accurate embedding, it is an unstable one."""
    if len(a) != len(b) or len(a) < 4:
        return None
    n = len(a)
    labels_a, labels_b = sorted(set(a)), sorted(set(b))
    table = np.zeros((len(labels_a), len(labels_b)), dtype=np.int64)
    ia = {l: i for i, l in enumerate(labels_a)}
    ib = {l: i for i, l in enumerate(labels_b)}
    for x, y in zip(a, b):
        table[ia[x], ib[y]] += 1

    def comb2(x):
        return x * (x - 1) / 2

    sum_ij = comb2(table).sum()
    sum_i = comb2(table.sum(axis=1)).sum()
    sum_j = comb2(table.sum(axis=0)).sum()
    total = comb2(np.array(n, dtype=np.float64))
    expected = sum_i * sum_j / total if total else 0.0
    maximum = (sum_i + sum_j) / 2
    denom = maximum - expected
    return float((sum_ij - expected) / denom) if denom != 0 else 1.0

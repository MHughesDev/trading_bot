"""The `InstrumentProfile` job's sidecar half (SPEC 5.2-5.3, checklist 3.1/3.4).

`fingerprint.compute` describes one instrument and `embedding.fit_whitener`
describes a population; this module is the thing that turns a batch of
instruments into rows the knowledge plane can store. It computes nothing new —
every statistic here comes from one of those two modules — and it exists because
three decisions have to be made *between* them, and each has a wrong answer that
is invisible once written.

**The dimension order is declared, not discovered.** `tier1` is a bare
`REAL[]` in the database: position *is* meaning. Deriving the order from
`sorted(fp.values)` would work perfectly until an instrument is missing a
dimension, at which point every position after it shifts and two rows written a
day apart stop describing the same thing. `TIER1_DIMENSIONS` is the contract, it
is checked against `fingerprint.compute`'s own output by a test, and a
dimension nothing measured is stored as SQL `NULL` in its own slot.

**Absent stays absent in `tier1`, and is imputed only for the retrieval
vector.** A whitening transform cannot consume a hole, so for the *projection*
an absent dimension takes the population median and the row is flagged. The
stored `tier1` keeps the `None`. The two are different claims: "we do not know
this instrument's Kyle lambda" and "we placed it at the population's median for
the purpose of taking a distance" — and a consumer reading `tier1` must see the
first.

**A population too small to whiten does not silently produce a vector
anyway.** `fit_whitener` refuses below 60 instruments, because a covariance
estimated from fewer points than it has dimensions yields components that are
noise. When it refuses, this module does *not* substitute a quietly different
metric under the same name: it returns `whitened = False`, sets
`QUALITY_UNWHITENED` on every row, and the caller is expected to store the batch
under an `embedding_version` whose name says so. 3.11 is what promotes a version
to the default, and an unwhitened one cannot pass it.
"""

from __future__ import annotations

import warnings
from dataclasses import dataclass
from typing import Any

import numpy as np

from . import embedding
from .fingerprint import ACF_LAGS, RV_HORIZONS, VR_HORIZONS, Bars, Fingerprint, compute

# The stored order of `knowledge.asset_embedding.tier1`. Position is meaning;
# see the module docstring. Appending is safe, reordering is not.
TIER1_DIMENSIONS: tuple[str, ...] = (
    *(f"rv_{h}" for h in RV_HORIZONS),
    "vol_of_vol",
    "skew",
    "kurtosis",
    "downside_vol",
    "upside_vol",
    *(f"acf_{lag}" for lag in ACF_LAGS),
    *(f"vr_{q}" for q in VR_HORIZONS),
    "hurst",
    "jump_share",
    "amihud",
    "kyle_lambda",
    "edge_spread",
    "overnight_share",
    "turnover",
)

# ── quality flags ────────────────────────────────────────────────────────────
#
# A bitmask on the row, mirrored in `api::instrument_profile`. Each one is a
# reason to distrust the *retrieval vector* specifically; `tier1` is always the
# measurement, or NULL where there was none.

# The population was below `embedding.MIN_POPULATION`, so the projection is a
# per-dimension standardisation rather than PCA whitening.
QUALITY_UNWHITENED = 1 << 0
# At least one dimension was absent for this instrument and took the population
# median in the projection.
QUALITY_IMPUTED_DIMENSION = 1 << 1
# Fewer than half the declared dimensions could be measured at all.
QUALITY_SPARSE_FINGERPRINT = 1 << 2


@dataclass
class ProfiledInstrument:
    """One instrument's row, ready for `knowledge.asset_embedding`."""

    instrument_id: int
    venue_id: int
    # `None` where the dimension was not measured. Never 0.0.
    tier1: list[float | None]
    retrieval_vec: list[float]
    absent: dict[str, str]
    quality_flags: int

    def as_dict(self) -> dict[str, Any]:
        return {
            "instrument_id": self.instrument_id,
            "venue_id": self.venue_id,
            "tier1": self.tier1,
            "retrieval_vec": self.retrieval_vec,
            "absent": self.absent,
            "quality_flags": self.quality_flags,
        }


@dataclass
class ProfileBatch:
    """What one `InstrumentProfile` job produced."""

    instruments: list[ProfiledInstrument]
    whitened: bool
    # `None` below ten rows: the two-nearest-neighbour estimator has nothing to
    # estimate from, and a number invented there would be monitored as though it
    # meant something.
    intrinsic_dim: float | None
    # Why the population was not whitened, in words, when it was not.
    reason: str | None

    def as_dict(self) -> dict[str, Any]:
        return {
            "instruments": [i.as_dict() for i in self.instruments],
            "whitened": self.whitened,
            "intrinsic_dim": self.intrinsic_dim,
            "reason": self.reason,
            "dimensions": list(TIER1_DIMENSIONS),
        }


def tier1_vector(fp: Fingerprint) -> list[float | None]:
    """The fingerprint in the declared order, `None` where it was not measured."""
    return [fp.values.get(name) for name in TIER1_DIMENSIONS]


def _standardise(matrix: np.ndarray) -> np.ndarray:
    """Per-dimension standardisation, L2-normalised.

    The fallback when the population is too small to whiten. It is *not*
    whitening — it removes scale but not correlation, so two dimensions that
    move together still count twice in the distance. That is exactly why the
    rows it produces carry `QUALITY_UNWHITENED` rather than being handed out as
    though they came from the fitted transform.
    """
    mean = matrix.mean(axis=0)
    sd = np.clip(matrix.std(axis=0, ddof=0), 1e-12, None)
    z = (matrix - mean) / sd
    norms = np.linalg.norm(z, axis=1, keepdims=True)
    return z / np.clip(norms, 1e-12, None)


def _pad(vectors: np.ndarray, width: int = embedding.RETRIEVAL_DIMS) -> np.ndarray:
    """Pad to the column width with zeros.

    The tier-1 block has fewer dimensions than `vector(48)` holds, and the
    column width is fixed by the schema. Zeros are the honest filler for a
    cosine metric: a dimension that is zero for every row contributes nothing to
    any distance and nothing to any norm, so padding cannot move a neighbour
    list. It is not a measurement of anything, which is why it is safe here and
    would not be safe in `tier1`.
    """
    out = np.zeros((vectors.shape[0], width), dtype=np.float64)
    keep = min(width, vectors.shape[1])
    out[:, :keep] = vectors[:, :keep]
    return out


def profile_population(
    bars_by_instrument: list[tuple[int, int, Bars]],
    *,
    deflate: bool = True,
) -> ProfileBatch:
    """Fingerprint every instrument in a batch and project the population.

    `bars_by_instrument` is `(instrument_id, venue_id, bars)`. An instrument
    whose fingerprint measured nothing at all is dropped rather than written as a
    row of nulls — a row that says nothing is not knowledge, and it would still
    occupy a cell in every neighbour search.
    """
    measured: list[tuple[int, int, Fingerprint]] = []
    for instrument_id, venue_id, bars in bars_by_instrument:
        fp = compute(bars, deflate=deflate)
        if not fp.values:
            continue
        measured.append((instrument_id, venue_id, fp))

    if not measured:
        return ProfileBatch(
            instruments=[],
            whitened=False,
            intrinsic_dim=None,
            reason="no instrument in the batch had enough bars to measure anything",
        )

    tier1 = [tier1_vector(fp) for _, _, fp in measured]

    # The imputation matrix: population median per dimension, used only for the
    # projection. A dimension no instrument measured falls back to 0.0 — with
    # nothing to compare against, every row sits at the same place on that axis
    # and it contributes nothing to any distance.
    raw = np.array(
        [[np.nan if v is None else float(v) for v in row] for row in tier1],
        dtype=np.float64,
    )
    with warnings.catch_warnings():
        # A dimension no instrument in the batch measured is an all-NaN slice.
        # That is a fact about the population, not an error: the fallback below
        # puts every row at the same place on that axis, where it contributes
        # nothing to any distance.
        warnings.simplefilter("ignore", RuntimeWarning)
        medians = np.nanmedian(raw, axis=0)
    medians = np.where(np.isfinite(medians), medians, 0.0)
    filled = np.where(np.isfinite(raw), raw, medians)
    # A dimension can still be non-finite if the fingerprint produced an inf.
    filled = np.where(np.isfinite(filled), filled, 0.0)

    whitened = False
    reason: str | None = None
    try:
        whitener = embedding.fit_whitener(filled)
        projected = whitener.transform(filled)
        whitened = True
    except embedding.NotEnoughPopulation as e:
        reason = str(e)
        projected = _standardise(filled)

    vectors = _pad(np.atleast_2d(projected))
    intrinsic = embedding.intrinsic_dimension(vectors)

    rows: list[ProfiledInstrument] = []
    for i, (instrument_id, venue_id, fp) in enumerate(measured):
        flags = 0
        if not whitened:
            flags |= QUALITY_UNWHITENED
        if any(v is None for v in tier1[i]):
            flags |= QUALITY_IMPUTED_DIMENSION
        if sum(v is not None for v in tier1[i]) * 2 < len(TIER1_DIMENSIONS):
            flags |= QUALITY_SPARSE_FINGERPRINT
        rows.append(
            ProfiledInstrument(
                instrument_id=instrument_id,
                venue_id=venue_id,
                tier1=tier1[i],
                retrieval_vec=[float(x) for x in vectors[i]],
                absent=dict(fp.absent),
                quality_flags=flags,
            )
        )

    return ProfileBatch(
        instruments=rows,
        whitened=whitened,
        intrinsic_dim=intrinsic,
        reason=reason,
    )

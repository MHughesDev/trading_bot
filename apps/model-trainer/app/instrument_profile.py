"""The two calls the platform's `instrument_profile` job makes (SPEC 5.2-5.3,
checklist 3.1/3.4, ADR-P3-01/05).

`fingerprint.compute` and `embedding.fit_whitener` were complete, tested and had
no caller: nothing sent them bars, so `knowledge.asset_embedding` was never
written. This module is the HTTP-shaped door they were missing, and nothing else;
the statistics stay where they were.

Two calls, not one, because they have different populations. A fingerprint is a
property of one instrument and can be computed the day it has bars. An embedding
is a property of the *population* - the whitener is fitted over every instrument
- and refuses below `MIN_POPULATION`. So the job stores fingerprints as it
computes them and asks for an embedding when it has enough of them; a platform
with twelve instruments has twelve fingerprints and no embeddings, and says so.
"""

from __future__ import annotations

from typing import Optional

import numpy as np
from pydantic import BaseModel

from . import embedding, fingerprint

# Every dimension `fingerprint.compute` can produce, in one declared order. A
# fingerprint is stored as an array in this order with NULL for an absent
# dimension (NULL is "not measured"; zero is a measurement).
TIER1_ORDER: tuple[str, ...] = tuple(
    sorted(
        [f"rv_{h}" for h in fingerprint.RV_HORIZONS]
        + [f"acf_{lag}" for lag in fingerprint.ACF_LAGS]
        + [f"vr_{q}" for q in fingerprint.VR_HORIZONS]
        + [
            "vol_of_vol",
            "skew",
            "kurtosis",
            "downside_vol",
            "upside_vol",
            "hurst",
            "jump_share",
            "amihud",
            "kyle_lambda",
            "edge_spread",
            "overnight_share",
            "turnover",
        ]
    )
)

# The version string the platform writes beside the vectors. Bumped when the
# order, the whitening, or the reduction changes.
EMBEDDING_VERSION = "tier1_pca48_v1"

# `knowledge.asset_embedding.retrieval_vec` is vector(48). Whitening yields
# min(48, n_features) components; the remainder is zero-padded, which changes no
# cosine similarity between two padded vectors.
VECTOR_WIDTH = embedding.RETRIEVAL_DIMS

# A column measured on fewer than this fraction of the population is dropped
# rather than imputed: imputing most of a column is inventing it.
MIN_COLUMN_COVERAGE = 0.5

# Bars below this and the fingerprint is mostly absent flags; the job skips the
# instrument and says so rather than storing a vector of NULLs.
MIN_BARS = 500


class BarsIn(BaseModel):
    instrument_id: int
    venue_id: int
    ts_ns: list[int]
    open: list[float]
    high: list[float]
    low: list[float]
    close: list[float]
    volume: list[float]
    signed_volume: Optional[list[float]] = None
    continuous_venue: bool = True


class FingerprintRequest(BaseModel):
    window_spec: str
    instruments: list[BarsIn]


class EmbedRequest(BaseModel):
    order: list[str]
    rows: list[list[Optional[float]]]


def fingerprint_instruments(req: FingerprintRequest) -> dict:
    """Tier-1 fingerprints for each instrument, in `TIER1_ORDER`."""
    out = []
    skipped = []
    for inst in req.instruments:
        n = len(inst.close)
        lengths = {len(inst.ts_ns), len(inst.open), len(inst.high), len(inst.low), len(inst.volume)}
        if lengths != {n}:
            skipped.append(
                {
                    "instrument_id": inst.instrument_id,
                    "venue_id": inst.venue_id,
                    "reason": "the bar arrays are not the same length",
                }
            )
            continue
        if n < MIN_BARS:
            skipped.append(
                {
                    "instrument_id": inst.instrument_id,
                    "venue_id": inst.venue_id,
                    "reason": f"{n} bars, fewer than {MIN_BARS}",
                }
            )
            continue
        bars = fingerprint.Bars(
            ts_ns=np.asarray(inst.ts_ns, dtype=np.int64),
            open=np.asarray(inst.open, dtype=np.float64),
            high=np.asarray(inst.high, dtype=np.float64),
            low=np.asarray(inst.low, dtype=np.float64),
            close=np.asarray(inst.close, dtype=np.float64),
            volume=np.asarray(inst.volume, dtype=np.float64),
            signed_volume=(
                None
                if inst.signed_volume is None
                else np.asarray(inst.signed_volume, dtype=np.float64)
            ),
            continuous_venue=inst.continuous_venue,
        )
        fp = fingerprint.compute(bars)
        unknown = set(fp.values) - set(TIER1_ORDER)
        if unknown:
            # A dimension the order does not name would be silently dropped from
            # the stored vector. Refuse loudly: the order is the schema.
            raise ValueError(
                f"fingerprint produced dimensions TIER1_ORDER lacks: {sorted(unknown)}"
            )
        out.append(
            {
                "instrument_id": inst.instrument_id,
                "venue_id": inst.venue_id,
                "bars_used": n,
                "tier1": fp.vector(TIER1_ORDER),
                "absent": fp.absent,
            }
        )
    return {
        "window_spec": req.window_spec,
        "order": list(TIER1_ORDER),
        "fingerprints": out,
        "skipped": skipped,
    }


def embed_population(req: EmbedRequest) -> dict:
    """Whiten and reduce a population's tier-1 rows to retrieval vectors.

    # Raises
    `embedding.NotEnoughPopulation` below `MIN_POPULATION` rows, which the HTTP
    layer reports as a 422 with the reason, because "not enough instruments yet"
    is an answer the job has to carry rather than a failure.
    """
    width = len(req.order)
    x = np.full((len(req.rows), width), np.nan, dtype=np.float64)
    for i, row in enumerate(req.rows):
        if len(row) != width:
            raise ValueError(f"row {i} has {len(row)} values for {width} dimensions")
        for j, v in enumerate(row):
            if v is not None:
                x[i, j] = float(v)

    coverage = np.isfinite(x).mean(axis=0) if len(x) else np.zeros(width)
    keep = coverage >= MIN_COLUMN_COVERAGE
    dropped = [name for name, k in zip(req.order, keep) if not k]
    x = x[:, keep]
    kept = [name for name, k in zip(req.order, keep) if k]
    if x.shape[1] == 0:
        raise ValueError("no dimension is measured on enough of the population to embed")

    # Impute the rest with the column median and *count* it per row, so a row that
    # is mostly imputation is visible in its quality flags.
    medians = np.nanmedian(x, axis=0)
    imputed_per_row = (~np.isfinite(x)).sum(axis=1)
    idx = np.where(~np.isfinite(x))
    x[idx] = np.take(medians, idx[1])

    # Standardise per column first: the block mixes a Hurst exponent near 0.5 with
    # an Amihud illiquidity near 1e-9, and SVD over raw columns is an SVD of the
    # largest-scale one.
    mu = x.mean(axis=0)
    sd = x.std(axis=0, ddof=1)
    sd = np.where(sd > 0, sd, 1.0)
    z = (x - mu) / sd

    whitener = embedding.fit_whitener(z)  # raises NotEnoughPopulation
    v = whitener.transform(z)
    if v.shape[1] < VECTOR_WIDTH:
        v = np.pad(v, ((0, 0), (0, VECTOR_WIDTH - v.shape[1])))
    return {
        "embedding_version": EMBEDDING_VERSION,
        "dimensions_used": kept,
        "dimensions_dropped": dropped,
        "retrieval": v.tolist(),
        "imputed_per_row": imputed_per_row.tolist(),
        "intrinsic_dim": embedding.intrinsic_dimension(v[:, : min(len(kept), VECTOR_WIDTH)]),
    }

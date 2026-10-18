"""The `instrument_profile` job's two calls (SPEC 5.2-5.3, checklist 3.1/3.4)."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import embedding, fingerprint, instrument_profile as profile  # noqa: E402

MIN = 60_000_000_000


def bars_in(instrument_id, n=900, seed=1, vol=0.001):
    r = np.random.default_rng(seed)
    rets = r.normal(0, vol, n)
    close = 100 * np.exp(np.cumsum(rets))
    return profile.BarsIn(
        instrument_id=instrument_id,
        venue_id=1,
        ts_ns=[int(i * MIN) for i in range(n)],
        open=[float(x) for x in np.concatenate([[100.0], close[:-1]])],
        high=[float(x) for x in close * 1.0005],
        low=[float(x) for x in close * 0.9995],
        close=[float(x) for x in close],
        volume=[1000.0 + float(x) for x in r.uniform(0, 100, n)],
    )


def test_every_dimension_the_fingerprint_can_produce_has_a_place_in_the_order():
    """If `fingerprint.compute` grows a dimension, the order must name it, or the
    stored vector silently drops it. `fingerprint_instruments` refuses; this
    catches it before a job does."""
    produced = set()
    for seed, signed in ((1, False), (2, True)):
        r = np.random.default_rng(seed)
        n = 900
        close = 100 * np.exp(np.cumsum(r.normal(0, 0.01, n)))
        bars = fingerprint.Bars(
            ts_ns=np.arange(n, dtype=np.int64) * 3_600_000_000_000,
            open=np.concatenate([[100.0], close[:-1]]),
            high=close * 1.001,
            low=close * 0.999,
            close=close,
            volume=np.full(n, 500.0),
            signed_volume=r.normal(0, 50, n) if signed else None,
            continuous_venue=signed,
        )
        fp = fingerprint.compute(bars)
        produced |= set(fp.values) | set(fp.absent)
    assert produced <= set(profile.TIER1_ORDER), produced - set(profile.TIER1_ORDER)


def test_fingerprints_come_back_in_the_declared_order_with_null_for_absent():
    out = profile.fingerprint_instruments(
        profile.FingerprintRequest(window_spec="21d@1m", instruments=[bars_in(1)])
    )
    assert out["order"] == list(profile.TIER1_ORDER)
    (fp,) = out["fingerprints"]
    assert len(fp["tier1"]) == len(profile.TIER1_ORDER)
    for name, value in zip(profile.TIER1_ORDER, fp["tier1"]):
        if name in fp["absent"]:
            assert value is None, f"{name} is absent and must be null, not zero"
        else:
            assert value is not None and np.isfinite(value), name
    # A venue that publishes no trade sides has no Kyle lambda: null, with a reason.
    assert "kyle_lambda" in fp["absent"] and fp["absent"]["kyle_lambda"]


def test_a_thin_or_ragged_instrument_is_skipped_with_a_reason_not_stored_as_nulls():
    thin = bars_in(2, n=100)
    ragged = bars_in(3)
    ragged.volume = ragged.volume[:-1]
    out = profile.fingerprint_instruments(
        profile.FingerprintRequest(window_spec="21d@1m", instruments=[thin, ragged])
    )
    assert out["fingerprints"] == []
    reasons = {s["instrument_id"]: s["reason"] for s in out["skipped"]}
    assert "fewer than" in reasons[2]
    assert "same length" in reasons[3]


def population(n, width=len(profile.TIER1_ORDER), seed=0, holes=0.0):
    r = np.random.default_rng(seed)
    x = r.normal(size=(n, width)) * (10.0 ** r.integers(-9, 3, size=width))
    rows = [[float(v) for v in row] for row in x]
    if holes:
        for row in rows:
            for j in range(width):
                if r.random() < holes:
                    row[j] = None
    return rows


def test_a_small_population_is_refused_by_name():
    with pytest.raises(embedding.NotEnoughPopulation):
        profile.embed_population(
            profile.EmbedRequest(order=list(profile.TIER1_ORDER), rows=population(12))
        )


def test_a_population_embeds_to_unit_vectors_of_the_columns_width():
    out = profile.embed_population(
        profile.EmbedRequest(order=list(profile.TIER1_ORDER), rows=population(80))
    )
    v = np.asarray(out["retrieval"])
    assert v.shape == (80, profile.VECTOR_WIDTH)
    norms = np.linalg.norm(v, axis=1)
    assert np.allclose(norms, 1.0, atol=1e-6), "the retrieval metric is cosine"
    # Padding is zeros, so it does not move any similarity.
    assert np.allclose(v[:, len(profile.TIER1_ORDER):], 0.0)
    assert out["embedding_version"] == profile.EMBEDDING_VERSION


def test_imputation_is_counted_per_row_and_a_mostly_absent_column_is_dropped():
    rows = population(80, holes=0.05, seed=3)
    # One column absent in 70 of 80 rows: imputing most of it is inventing it.
    for i in range(70):
        rows[i][0] = None
    out = profile.embed_population(
        profile.EmbedRequest(order=list(profile.TIER1_ORDER), rows=rows)
    )
    assert profile.TIER1_ORDER[0] in out["dimensions_dropped"]
    assert len(out["imputed_per_row"]) == 80
    assert sum(out["imputed_per_row"]) > 0


def test_a_ragged_row_is_an_error():
    rows = population(70)
    rows[5] = rows[5][:-1]
    with pytest.raises(ValueError, match="row 5"):
        profile.embed_population(
            profile.EmbedRequest(order=list(profile.TIER1_ORDER), rows=rows)
        )


def test_the_http_surface_reports_a_small_population_as_422_with_the_reason():
    pytest.importorskip("nats", reason="app.main imports the NATS publisher")
    from fastapi.testclient import TestClient

    from app.main import app

    client = TestClient(app)
    r = client.post(
        "/embed", json={"order": list(profile.TIER1_ORDER), "rows": population(10)}
    )
    assert r.status_code == 422
    assert r.json()["detail"]["error"] == "not_enough_population"
    ok = client.post(
        "/fingerprint",
        json={"window_spec": "21d@1m", "instruments": [bars_in(1).model_dump()]},
    )
    assert ok.status_code == 200
    assert ok.json()["fingerprints"][0]["instrument_id"] == 1

"""Random-projection feature embedding: determinism, normalization, and meaningful geometry."""

from __future__ import annotations

import math

from data_plane.memory.embeddings import feature_dict_to_embedding


def _cos(a: list[float], b: list[float]) -> float:
    return sum(x * y for x, y in zip(a, b, strict=True))


def test_deterministic_and_unit_norm() -> None:
    f = {"rsi_14": 0.7, "macd": -0.2, "vol_15": 0.05}
    e1 = feature_dict_to_embedding(f, dim=64)
    e2 = feature_dict_to_embedding(dict(f), dim=64)
    assert e1 == e2
    assert len(e1) == 64
    assert math.isclose(math.sqrt(sum(x * x for x in e1)), 1.0, rel_tol=1e-9)


def test_similar_inputs_more_similar_than_dissimilar() -> None:
    base = {"rsi_14": 0.70, "macd": 0.10, "vol_15": 0.04}
    near = {"rsi_14": 0.72, "macd": 0.11, "vol_15": 0.04}
    far = {"rsi_14": -0.80, "macd": -0.50, "vol_15": 0.90}
    eb = feature_dict_to_embedding(base)
    en = feature_dict_to_embedding(near)
    ef = feature_dict_to_embedding(far)
    # A nearby feature vector should be much closer in cosine than a very different one.
    assert _cos(eb, en) > _cos(eb, ef)
    assert _cos(eb, en) > 0.9


def test_booleans_and_nonfinite_ignored() -> None:
    e = feature_dict_to_embedding(
        {"a": 1.0, "flag": True, "bad": float("nan"), "inf": float("inf")}
    )
    # Only "a" contributes; result is a finite unit vector.
    assert math.isclose(math.sqrt(sum(x * x for x in e)), 1.0, rel_tol=1e-9)
    assert all(math.isfinite(x) for x in e)


def test_empty_returns_zero_vector() -> None:
    assert feature_dict_to_embedding({}, dim=32) == [0.0] * 32
    assert feature_dict_to_embedding({"only_text": "x"}) == [0.0] * 64  # type: ignore[dict-item]


def test_dim_is_respected() -> None:
    assert len(feature_dict_to_embedding({"x": 1.0}, dim=16)) == 16
    assert len(feature_dict_to_embedding({"x": 1.0}, dim=128)) == 128

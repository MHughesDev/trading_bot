"""An embedding version answers with its vector (Set U U-5.2)."""
from __future__ import annotations

import asyncio
import json
import os
import pathlib
import sys

import httpx
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app.predictors.embed_predictor import embed  # noqa: E402
from app.schemas import EmbedRequest, EmbedRow  # noqa: E402


def request(texts, adapter=None):
    return EmbedRequest(
        model_id="emb",
        version=1,
        adapter=adapter if adapter is not None else {"provider": "ollama", "model": "nomic-embed-text"},
        rows=[EmbedRow(key=str(i), text=t) for i, t in enumerate(texts)],
    )


def fake(handler):
    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


def run(req, client=None):
    return asyncio.run(embed(req, client))


def test_each_text_gets_its_own_vector_as_plain_floats():
    seen = {}

    def handler(r):
        seen["url"] = str(r.url)
        seen["body"] = r.read()
        return httpx.Response(200, json={"embeddings": [[0.1, -0.2, 0.3], [1.0, 2.0, 3.0]]})

    out = run(request(["fed hikes", "fed cuts"]), fake(handler))
    assert seen["url"] == "http://localhost:11434/api/embed"
    assert json.loads(seen["body"]) == {"model": "nomic-embed-text", "input": ["fed hikes", "fed cuts"]}
    assert [a.status for a in out.answers] == ["answered", "answered"]
    assert out.answers[0].outputs == {"embedding": [0.1, -0.2, 0.3]}
    assert [a.key for a in out.answers] == ["0", "1"]


def test_a_count_mismatch_or_failure_is_unavailable_never_a_guess():
    out = run(request(["a", "b"]), fake(lambda r: httpx.Response(200, json={"embeddings": [[1.0]]})))
    assert {a.status for a in out.answers} == {"unavailable"}
    assert "1 vectors for 2 texts" in out.answers[0].detail
    out = run(request(["a"]), fake(lambda r: httpx.Response(500, text="boom")))
    assert out.answers[0].status == "unavailable"


def test_a_non_finite_vector_abstains():
    raw = b'{"embeddings": [[1.0, NaN]]}'
    out = run(request(["a"]), fake(lambda r: httpx.Response(200, content=raw)))
    assert out.answers[0].status == "abstained"


def test_an_unknown_provider_or_missing_model_is_named():
    out = run(request(["a"], {"provider": "somewhere", "model": "x"}))
    assert "not served here" in out.answers[0].detail
    out = run(request(["a"], {"provider": "ollama"}))
    assert "names no embedding model" in out.answers[0].detail


@pytest.mark.skipif(not os.environ.get("EMBED_LIVE"), reason="set EMBED_LIVE=1 with Ollama serving nomic-embed-text")
def test_the_local_model_embeds_and_similar_texts_are_closer():
    out = run(request([
        "the central bank raised interest rates",
        "the fed hiked rates again",
        "a recipe for sourdough bread",
    ]))
    vecs = [a.outputs["embedding"] for a in out.answers]
    assert all(a.status == "answered" for a in out.answers)
    assert len({len(v) for v in vecs}) == 1 and len(vecs[0]) > 100

    def cos(a, b):
        dot = sum(x * y for x, y in zip(a, b))
        return dot / (sum(x * x for x in a) ** 0.5 * sum(y * y for y in b) ** 0.5)

    assert cos(vecs[0], vecs[1]) > cos(vecs[0], vecs[2])

"""Embedding versions answer with their vector (Set U U-5.2).

An `embedding` model version is an adapter, like an external LLM: its
definition names a provider and a model, and its contract is `text` in,
`embedding: vec` out. The answer is the provider's vector as plain floats —
never padded, truncated or normalised here: the width is whatever the model
produces, and a strategy reads it as that.

Only local providers are served. `ollama` runs on this machine with no key, so
an embedding costs nothing and leaves nothing. A provider this service does not
know is `unavailable` by name rather than a guess at an API.
"""
from __future__ import annotations

import math

import httpx

from ..schemas import EmbedRequest, EmbedResponse, V2Answer

PROVIDERS = ("ollama",)


async def embed(req: EmbedRequest, client: httpx.AsyncClient | None = None) -> EmbedResponse:
    adapter = req.adapter or {}
    provider = (adapter.get("provider") or "ollama").lower()
    model = adapter.get("model") or ""
    if provider not in PROVIDERS:
        return _all(req, "unavailable", f"embedding provider {provider!r} is not served here")
    if not model:
        return _all(req, "unavailable", "the adapter names no embedding model")
    url = (adapter.get("endpoint") or "http://localhost:11434").rstrip("/") + "/api/embed"
    own = client is None
    client = client or httpx.AsyncClient(timeout=60.0)
    try:
        r = await client.post(url, json={"model": model, "input": [row.text for row in req.rows]})
        r.raise_for_status()
        vectors = r.json().get("embeddings") or []
    except Exception as e:  # noqa: BLE001
        return _all(req, "unavailable", f"{provider} embedding failed: {e}")
    finally:
        if own:
            await client.aclose()
    if len(vectors) != len(req.rows):
        return _all(
            req,
            "unavailable",
            f"{provider} returned {len(vectors)} vectors for {len(req.rows)} texts",
        )
    answers = []
    for row, vec in zip(req.rows, vectors):
        values = [float(x) for x in vec]
        if not values or not all(math.isfinite(x) for x in values):
            answers.append(V2Answer(key=row.key, status="abstained", outputs={},
                                    detail="the embedding is empty or not finite"))
        else:
            answers.append(V2Answer(key=row.key, status="answered", outputs={"embedding": values}))
    return EmbedResponse(model_id=req.model_id, version=req.version, answers=answers)


def _all(req: EmbedRequest, status: str, detail: str) -> EmbedResponse:
    return EmbedResponse(
        model_id=req.model_id,
        version=req.version,
        answers=[V2Answer(key=row.key, status=status, outputs={}, detail=detail) for row in req.rows],
    )

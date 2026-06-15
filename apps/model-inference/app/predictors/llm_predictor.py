import time
import uuid
from decimal import Decimal

import httpx

from ..schemas import LLMPredictRequest, LLMPredictResponse


async def predict_llm(req: LLMPredictRequest) -> LLMPredictResponse:
    start = time.time()
    adapter = req.adapter or {}
    provider = (adapter.get("provider") or "ollama").lower()
    model = adapter.get("model", "")
    endpoint = adapter.get("endpoint", "")
    params = {**(adapter.get("default_params") or {}), **(req.params or {})}
    cost_per_1k = Decimal(str(adapter.get("cost_per_1k_tokens", "0")))

    text = ""
    tokens = 0
    trace_id = str(uuid.uuid4())

    async with httpx.AsyncClient(timeout=120.0) as client:
        if provider == "ollama":
            url = (endpoint or "http://localhost:11434").rstrip("/") + "/api/generate"
            r = await client.post(url, json={"model": model, "prompt": req.prompt, "stream": False, "options": params})
            r.raise_for_status()
            data = r.json()
            text = data.get("response", "")
            tokens = int(data.get("eval_count", 0)) + int(data.get("prompt_eval_count", 0))
        elif provider == "openai":
            import os
            url = (endpoint or "https://api.openai.com/v1").rstrip("/") + "/chat/completions"
            headers = {"Authorization": f"Bearer {os.environ.get('OPENAI_API_KEY', '')}"}
            r = await client.post(url, headers=headers, json={"model": model, "messages": [{"role": "user", "content": req.prompt}], **params})
            r.raise_for_status()
            data = r.json()
            text = data["choices"][0]["message"]["content"]
            tokens = int(data.get("usage", {}).get("total_tokens", 0))
        elif provider == "anthropic":
            import os
            url = (endpoint or "https://api.anthropic.com/v1").rstrip("/") + "/messages"
            headers = {
                "x-api-key": os.environ.get("ANTHROPIC_API_KEY", ""),
                "anthropic-version": "2023-06-01",
            }
            max_tokens = int(params.pop("max_tokens", 1024))
            r = await client.post(url, headers=headers, json={"model": model, "max_tokens": max_tokens, "messages": [{"role": "user", "content": req.prompt}], **params})
            r.raise_for_status()
            data = r.json()
            text = "".join(block.get("text", "") for block in data.get("content", []))
            usage = data.get("usage", {})
            tokens = int(usage.get("input_tokens", 0)) + int(usage.get("output_tokens", 0))
        else:
            raise ValueError(f"unsupported provider: {provider}")

    latency_ms = int((time.time() - start) * 1000)
    cost = (cost_per_1k * Decimal(tokens) / Decimal(1000)).quantize(Decimal("0.000001"))
    return LLMPredictResponse(text=text, tokens=tokens, latency_ms=latency_ms, cost_usd=str(cost), trace_id=trace_id)

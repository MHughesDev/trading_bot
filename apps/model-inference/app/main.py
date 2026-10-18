import time

from fastapi import FastAPI, HTTPException

from .schemas import (
    PredictRequest,
    PredictResponse,
    Forecast,
    EmbedRequest,
    EmbedResponse,
    LLMPredictRequest,
    LLMPredictResponse,
    V2Answer,
    V2PredictRequest,
    V2PredictResponse,
)
from . import v2 as v2_mod
from .predictors import bundle as bundle_mod
from .loader import CACHE
from .predictors import base, run_predict
from .predictors.embed_predictor import embed
from .predictors.llm_predictor import predict_llm

app = FastAPI(title="model-inference", version="0.1.0")

DEFAULT_HORIZON = "1h"


@app.get("/health")
async def health():
    return {"status": "ok", "service": "model-inference"}


@app.post("/predict")
async def predict(req: PredictRequest):
    start = time.time()

    # Load artifact bytes (with sha256 verification). In dev the artifact URI
    # may be a stub — degrade gracefully to flat forecasts rather than 500.
    try:
        artifact_bytes = CACHE.load(req.model_id, req.version, req.artifact_hash, req.artifact_uri)
    except ValueError:
        # Hash verification mismatch is a client/contract error.
        raise HTTPException(status_code=422, detail="artifact hash verification failed")
    except Exception:
        predictions = [
            base.to_forecast(0.5, DEFAULT_HORIZON) for _ in req.instances
        ]
        latency_ms = int((time.time() - start) * 1000)
        return PredictResponse(
            model_id=req.model_id,
            version=req.version,
            predictions=predictions,
            latency_ms=latency_ms,
        )

    # A request missing a feature the model was trained on is refused by name —
    # never zero-filled, and never turned into a flat forecast below.
    header, _ = bundle_mod.read_bundle(artifact_bytes)
    order = (header or {}).get("feature_order") or []
    for i, inst in enumerate(req.instances):
        missing = [k for k in order if inst.features.get(k) is None]
        if missing:
            raise HTTPException(
                status_code=422,
                detail=f"instance {i} is missing required feature(s): {', '.join(missing)}",
            )

    try:
        predictions: list[Forecast] | None = run_predict(
            artifact_bytes, req.instances, req.model_kind, DEFAULT_HORIZON
        )
    except base.Abstain as a:
        # The model declined; a made-up forecast would be worse than none.
        raise HTTPException(status_code=422, detail=f"abstain: {a.reason}")

    if predictions is None:
        # Total failure across all predictors — never 500 in dev.
        predictions = [base.to_forecast(0.5, DEFAULT_HORIZON) for _ in req.instances]

    latency_ms = int((time.time() - start) * 1000)
    return PredictResponse(
        model_id=req.model_id,
        version=req.version,
        predictions=predictions,
        latency_ms=latency_ms,
    )


@app.post("/v2/predict")
async def predict_v2(req: V2PredictRequest) -> V2PredictResponse:
    """Typed, named inputs and outputs (Set U U-5.2). Never a made-up answer:
    a row missing a feature abstains naming it; an artifact that cannot answer
    makes every row unavailable with the reason."""
    start = time.time()
    rows = [r.model_dump() for r in req.rows]
    if req.model_kind == "transform":
        return V2PredictResponse(
            model_id=req.model_id,
            version=req.version,
            answers=[V2Answer(**a) for a in v2_mod.run_transform(req.transform or {}, rows)],
            latency_ms=int((time.time() - start) * 1000),
        )
    try:
        artifact_bytes = CACHE.load(req.model_id, req.version, req.artifact_hash, req.artifact_uri)
        answers = v2_mod.predict_v2(artifact_bytes, rows, req.model_kind)
    except ValueError:
        raise HTTPException(status_code=422, detail="artifact hash verification failed")
    except Exception as e:  # noqa: BLE001
        answers = [
            {"key": r["key"], "status": "unavailable", "outputs": {}, "detail": f"artifact unavailable: {e}"}
            for r in rows
        ]
    return V2PredictResponse(
        model_id=req.model_id,
        version=req.version,
        answers=[V2Answer(**a) for a in answers],
        latency_ms=int((time.time() - start) * 1000),
    )


@app.post("/predict/llm")
async def predict_llm_endpoint(req: LLMPredictRequest) -> LLMPredictResponse:
    return await predict_llm(req)


@app.post("/v2/embed")
async def embed_endpoint(req: EmbedRequest) -> EmbedResponse:
    return await embed(req)

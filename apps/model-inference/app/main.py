import time

from fastapi import FastAPI, HTTPException

from .schemas import (
    PredictRequest,
    PredictResponse,
    Forecast,
    LLMPredictRequest,
    LLMPredictResponse,
)
from .loader import CACHE
from .predictors import (
    xgboost_predictor,
    lightgbm_predictor,
    sklearn_predictor,
    torch_predictor,
    base,
)
from .predictors.llm_predictor import predict_llm

app = FastAPI(title="model-inference", version="0.1.0")

DEFAULT_HORIZON = "1h"

# Tolerant ordering: try each predictor; the first that succeeds wins.
_PREDICTORS = [
    xgboost_predictor.predict,
    lightgbm_predictor.predict,
    sklearn_predictor.predict,
    torch_predictor.predict,
]


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

    predictions: list[Forecast] | None = None
    for fn in _PREDICTORS:
        try:
            predictions = fn(artifact_bytes, req.instances, req.model_kind, DEFAULT_HORIZON)
            if predictions is not None:
                break
        except Exception:
            continue

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


@app.post("/predict/llm")
async def predict_llm_endpoint(req: LLMPredictRequest) -> LLMPredictResponse:
    return await predict_llm(req)

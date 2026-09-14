import asyncio

from fastapi import FastAPI

from .schemas import TrainRequest, EvalRequest, PostHocRequest
from .worker import run_training, run_evaluation, run_posthoc, RESULTS

app = FastAPI(title="model-trainer", version="0.1.0")


@app.get("/health")
async def health():
    return {"status": "ok", "service": "model-trainer"}


@app.get("/capabilities")
async def capabilities():
    return {"frameworks": ["xgboost", "lightgbm", "sklearn", "torch"]}


@app.post("/train")
async def train(req: TrainRequest):
    result = await run_training(req)
    return result.model_dump()


@app.get("/train/{run_id}")
async def train_status(run_id: str):
    res = RESULTS.get(run_id)
    if res is None:
        return {"run_id": run_id, "status": "running"}
    return res.model_dump()


@app.post("/evaluate")
async def evaluate(req: EvalRequest):
    """Parity-preserving evaluation endpoint (I-2.1).

    Receives the artifact + test-window dataset; runs inference through the
    stored bundle (same path as serve); scores predicted distributions against
    realized outcomes; returns full metrics + scorecard + report.
    """
    result = await run_evaluation(req)
    return result.model_dump()


@app.post("/posthoc")
async def posthoc(req: PostHocRequest):
    """The post-hoc pipeline (SPEC 11.4, checklist 2.11).

    Soup -> greedy ensemble -> calibrate -> closed-form threshold, in that order,
    every time. There is no parameter that reorders or skips a step; a step that
    does not apply to the framework is returned as `skipped`, so a reader of the
    response can see the whole sequence rather than inferring it.
    """
    result = await run_posthoc(req)
    return result.model_dump()

import asyncio

from fastapi import FastAPI

from .schemas import TrainRequest
from .worker import run_training, RESULTS

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

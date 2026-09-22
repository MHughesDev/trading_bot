import asyncio

from fastapi import FastAPI

import numpy as np

from .schemas import TrainRequest, EvalRequest, PostHocRequest, ProfileRequest
from .worker import run_training, run_evaluation, run_posthoc, RESULTS
from . import fingerprint, profile

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


@app.post("/fingerprint")
async def instrument_profile(req: ProfileRequest):
    """Tier-1 fingerprints for a population (SPEC 5.2-5.3, checklist 3.1/3.4).

    The `InstrumentProfile` job's compute half. Bars in, one row per instrument
    out: the tier-1 vector in a declared order with `None` where a dimension
    could not be measured, and the 48-d retrieval vector the knowledge plane
    takes neighbours in.

    It is a *population* endpoint because whitening is a population-level fit
    (`embedding.fit_whitener`). Sending one instrument at a time would fit the
    transform on a sample of one, and below sixty instruments it refuses
    outright — in which case the response says `whitened: false` and every row
    carries `QUALITY_UNWHITENED`, rather than a differently-computed vector
    handed back under the same name.

    The endpoint writes nothing. Storage, `knowledge_time` and the choice of
    `embedding_version` all belong to the caller, which is the only side that
    knows what was knowable when.
    """
    batch = [
        (
            i.instrument_id,
            i.venue_id,
            fingerprint.Bars(
                ts_ns=np.asarray(i.bars.ts_ns, dtype=np.int64),
                open=np.asarray(i.bars.open, dtype=np.float64),
                high=np.asarray(i.bars.high, dtype=np.float64),
                low=np.asarray(i.bars.low, dtype=np.float64),
                close=np.asarray(i.bars.close, dtype=np.float64),
                volume=np.asarray(i.bars.volume, dtype=np.float64),
                signed_volume=(
                    None
                    if i.bars.signed_volume is None
                    else np.asarray(i.bars.signed_volume, dtype=np.float64)
                ),
                continuous_venue=i.bars.continuous_venue,
            ),
        )
        for i in req.instruments
    ]
    return profile.profile_population(batch, deflate=req.deflate).as_dict()

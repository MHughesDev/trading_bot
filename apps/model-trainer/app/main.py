import asyncio

from fastapi import FastAPI, HTTPException

from .schemas import TrainRequest, EvalRequest, PostHocRequest
from .worker import run_training, run_evaluation, run_posthoc, RESULTS
from . import embedding as _embedding
from . import instrument_profile as _profile

app = FastAPI(title="model-trainer", version="0.1.0")


@app.get("/health")
async def health():
    return {"status": "ok", "service": "model-trainer"}


@app.get("/capabilities")
async def capabilities():
    from . import checkpoint as _ckpt

    # The identity a checkpoint must match to be resumed into (`plan_resume`).
    return {
        "frameworks": ["xgboost", "lightgbm", "sklearn", "torch"],
        "code_hash": _ckpt.code_hash(),
        "image_digest": _ckpt.image_digest(),
    }


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
async def fingerprint(req: _profile.FingerprintRequest):
    """Tier-1 asset fingerprints (SPEC 5.2) - the `instrument_profile` job's call."""
    return _profile.fingerprint_instruments(req)


@app.post("/embed")
async def embed(req: _profile.EmbedRequest):
    """Whitened 48-d retrieval vectors over a population (SPEC 5.3).

    422 with the reason when the population is too small to whiten: "not enough
    instruments yet" is an answer the caller carries, not a failure.
    """
    try:
        return _profile.embed_population(req)
    except _embedding.NotEnoughPopulation as e:
        raise HTTPException(
            status_code=422, detail={"error": "not_enough_population", "message": str(e)}
        )
    except ValueError as e:
        raise HTTPException(status_code=422, detail={"error": "invalid", "message": str(e)})


# --- Set N (N-4.1): adaptive conformal calibration -------------------------------------
from . import calibration as _calibration  # noqa: E402


@app.post("/calibrate")
async def calibrate(req: _calibration.CalibrateRequest):
    """Wrap point (or band) predictions into calibrated intervals (ACI / ACI + CUSUM resets).

    Returns the bands, the coverage series and the reset indices; the Rust side calls this.
    """
    try:
        return _calibration.run(req)
    except _calibration.failures.DataError as e:
        raise HTTPException(status_code=422, detail={"error": "invalid", "message": str(e)})

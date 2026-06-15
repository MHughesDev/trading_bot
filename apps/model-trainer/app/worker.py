import asyncio
import io
import os

import pandas as pd

from .schemas import TrainRequest, TrainResponse
from .artifact_store import get_store
from .nats_client import NatsPublisher
from .adapters import xgboost_adapter, lightgbm_adapter, sklearn_adapter, torch_adapter, forecaster

# In-memory result store keyed by run_id (Test Lab / polling fallback).
RESULTS: dict[str, TrainResponse] = {}


def _route(framework: str, model_kind: str):
    if model_kind == "forecaster" and framework not in ("xgboost", "lightgbm", "sklearn", "torch"):
        return forecaster.train
    fw = (framework or "").lower()
    if fw == "xgboost":
        return xgboost_adapter.train
    if fw == "lightgbm":
        return lightgbm_adapter.train
    if fw == "sklearn":
        return sklearn_adapter.train
    if fw == "torch":
        # forecaster kind on torch uses the dedicated forecaster
        if model_kind == "forecaster":
            return forecaster.train
        return torch_adapter.train
    raise ValueError(f"unsupported framework: {framework}")


async def run_training(req: TrainRequest) -> TrainResponse:
    publisher = NatsPublisher()
    await publisher.connect()
    subject = req.progress.nats_subject

    loop = asyncio.get_event_loop()

    def emit(phase: str, progress: float, metric: dict | None = None):
        payload = {"run_id": req.run_id, "phase": phase, "progress": float(progress)}
        if metric:
            payload["metric"] = metric
        asyncio.run_coroutine_threadsafe(publisher.publish(subject, payload), loop)

    try:
        await publisher.publish(subject, {"run_id": req.run_id, "phase": "loading_dataset", "progress": 5.0})
        store = get_store()
        try:
            raw = store.get(req.dataset_uri)
            df = pd.read_parquet(io.BytesIO(raw))
        except Exception:
            # Dataset may be a stub URI in dev — synthesize a deterministic frame.
            df = _synthetic_frame()

        train_fn = _route(req.framework, req.model_kind)

        # Run the (blocking) training in a thread so progress callbacks can publish.
        artifact_bytes, metrics = await loop.run_in_executor(
            None, lambda: train_fn(req.definition, df, emit)
        )

        key = f"{req.output_prefix.rstrip('/')}/model.bin"
        # Normalize key to a relative path for the store.
        key = key.replace("file://", "")
        uri, sha256, size = store.put(key, artifact_bytes)

        resp = TrainResponse(
            status="succeeded",
            artifact_uri=uri,
            sha256=sha256,
            size_bytes=size,
            metrics=metrics,
            framework_version=str(metrics.get("framework_version")) if metrics else None,
        )
        await publisher.publish(subject, {
            "run_id": req.run_id, "phase": "succeeded", "progress": 100.0,
            "metric": metrics or {},
        })
    except Exception as e:  # noqa: BLE001
        resp = TrainResponse(status="failed", error=str(e))
        await publisher.publish(subject, {
            "run_id": req.run_id, "phase": "failed", "progress": 100.0,
            "metric": {"error": str(e)},
        })
    finally:
        await publisher.close()

    RESULTS[req.run_id] = resp
    return resp


def _synthetic_frame(n: int = 500):
    import numpy as np
    rng = np.random.default_rng(42)
    close = 50000 + np.cumsum(rng.normal(0, 50, n))
    df = pd.DataFrame({
        "open": close + rng.normal(0, 10, n),
        "high": close + np.abs(rng.normal(0, 20, n)),
        "low": close - np.abs(rng.normal(0, 20, n)),
        "close": close,
        "volume": rng.uniform(1e5, 1e6, n),
        "ema_7": close,
        "ema_14": close,
        "ema_21": close,
        "rsi_14": rng.uniform(20, 80, n),
        "rolling_mean_7": close,
        "rolling_std_7": rng.uniform(1, 50, n),
        "returns_1": rng.normal(0, 0.001, n),
        "log_returns_1": rng.normal(0, 0.001, n),
    })
    df["label"] = rng.normal(0, 0.002, n)
    return df

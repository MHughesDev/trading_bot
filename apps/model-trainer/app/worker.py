import asyncio
import io
import os

import pandas as pd

from .schemas import TrainRequest, TrainResponse
from .artifact_store import get_store
from .nats_client import NatsPublisher
from .clickhouse import fetch_bars
from .features import build_training_frame
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
        df = _load_dataframe(req)

        # Embargo gap (in bars) between train/val/test so a row's forward-return
        # label can't leak into the next split. Resolved from the data selection.
        if req.data is not None:
            from .features import horizon_in_bars

            req.definition["_embargo_bars"] = horizon_in_bars(
                req.data.label_horizon, req.data.timeframe
            )

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


def _load_dataframe(req: TrainRequest) -> pd.DataFrame:
    """Resolve the training frame.

    When the request carries a `data` selection, pull real bars from ClickHouse
    for each instrument, compute the requested features + forward-return label,
    and concatenate. Fails loudly if the selection yields no usable rows — we do
    NOT silently fall back to synthetic data when real data was requested.

    When there is no `data` selection (legacy/back-compat path), try the dataset
    URI and otherwise synthesize a deterministic frame.
    """
    if req.data is not None:
        spec = req.data
        frames: list[pd.DataFrame] = []
        for inst in spec.instruments:
            bars = fetch_bars(inst, spec.timeframe, spec.start, spec.end)
            frame = build_training_frame(
                bars, spec.features, spec.timeframe, spec.label_horizon
            )
            if not frame.empty:
                frames.append(frame)
        if not frames:
            raise ValueError(
                "no training rows from ClickHouse for "
                f"instruments={spec.instruments} timeframe={spec.timeframe} "
                f"window={spec.start}..{spec.end} — widen the lookback or pick an "
                "instrument with stored bars"
            )
        return pd.concat(frames, ignore_index=True)

    # Legacy path: no explicit selection.
    store = get_store()
    try:
        raw = store.get(req.dataset_uri)
        return pd.read_parquet(io.BytesIO(raw))
    except Exception:
        return _synthetic_frame()


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

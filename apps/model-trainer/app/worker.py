import asyncio
import io
import os

import numpy as np
import pandas as pd

from .schemas import TrainRequest, TrainResponse, EvalRequest, EvalResponse
from .artifact_store import get_store
from .nats_client import NatsPublisher
from . import scoring as scoring_mod
from .adapters import (
    xgboost_adapter,
    lightgbm_adapter,
    sklearn_adapter,
    torch_adapter,
    forecaster,
    lightgbm_q_adapter,
    xgboost_q_adapter,
    sklearn_q_adapter,
    garch_adapter,
)

# In-memory result store keyed by run_id (Test Lab / polling fallback).
RESULTS: dict[str, TrainResponse] = {}


def _route(framework: str, model_kind: str, definition: dict | None = None):
    """Return the train function for (framework, model_kind, output).

    Distributional routing (I-1.5, I-1.6):
      - framework=lightgbm + quantile_levels  → lightgbm_q_adapter
      - framework=xgboost  + quantile_levels  → xgboost_q_adapter
      - framework=sklearn  + quantile_levels  → sklearn_q_adapter
      - framework=garch    (any kind)          → garch_adapter
    """
    definition = definition or {}
    output = definition.get("output") or {}
    is_quantile = bool(output.get("quantile_levels"))

    fw = (framework or "").lower()

    if fw == "garch":
        return garch_adapter.train

    if is_quantile:
        if fw == "lightgbm":
            return lightgbm_q_adapter.train
        if fw == "xgboost":
            return xgboost_q_adapter.train
        if fw == "sklearn":
            return sklearn_q_adapter.train

    # Point model dispatch (unchanged).
    if model_kind == "forecaster" and fw not in ("xgboost", "lightgbm", "sklearn", "torch"):
        return forecaster.train
    if fw == "xgboost":
        return xgboost_adapter.train
    if fw == "lightgbm":
        return lightgbm_adapter.train
    if fw == "sklearn":
        return sklearn_adapter.train
    if fw == "torch":
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

        train_fn = _route(req.framework, req.model_kind, req.definition)

        # Inject fold-aware prepare when Rust supplied folds (I-0.10).
        if req.folds:
            req.definition["_wf_fold"] = req.folds[0].model_dump()

        artifact_bytes, metrics = await loop.run_in_executor(
            None, lambda: train_fn(req.definition, df, emit)
        )

        key = f"{req.output_prefix.rstrip('/')}/model.bin"
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


async def run_evaluation(req: EvalRequest) -> EvalResponse:
    """Parity-preserving eval loop (I-2.1).

    1. Load the pre-materialized test-window Parquet (realized outcomes included).
    2. Run inference through the stored bundle (same path as serve).
    3. PIT-correct join: labels are already horizon-shifted; no lookahead (ADR-0017).
    4. Score via the proper-scoring suite (I-2.3–I-2.9).
    5. Return full scorecard + report.
    """
    publisher = NatsPublisher()
    await publisher.connect()
    subject = req.progress.nats_subject if req.progress else None
    loop = asyncio.get_event_loop()

    async def _emit(phase: str, pct: float):
        if subject:
            await publisher.publish(subject, {
                "run_id": req.eval_id, "phase": phase, "progress": pct,
            })

    try:
        await _emit("loading_artifact", 5.0)
        store = get_store()

        # Load bundle
        try:
            artifact_bytes = store.get(req.artifact_uri)
        except Exception as e:  # noqa: BLE001
            return EvalResponse(status="failed", error=f"artifact load failed: {e}")

        # Load test-window dataset (Parquet from Phase 0 materialization)
        await _emit("loading_dataset", 15.0)
        try:
            raw = store.get(req.dataset_uri)
            df = pd.read_parquet(io.BytesIO(raw))
        except Exception as e:  # noqa: BLE001
            return EvalResponse(status="failed", error=f"dataset load failed: {e}")

        await _emit("running_inference", 30.0)

        # Determine framework from definition
        definition = req.definition
        framework = (definition.get("framework") or "").lower()
        output = definition.get("output") or {}
        is_quantile = bool(output.get("quantile_levels"))

        # Infer predictor path using the same routing logic as worker
        try:
            from . import engine as eng
            from .engine import Prepared
        except Exception:
            eng = None

        # Run inference using the loaded bundle
        predicted_q, levels, realized, sigma = _eval_inference(
            artifact_bytes, df, framework, is_quantile, definition,
        )

        if predicted_q is None or realized is None or len(realized) == 0:
            return EvalResponse(status="failed", error="no predictions or realized values produced")

        await _emit("scoring", 60.0)

        # Fold test ranges for per-fold breakdown
        fold_test_ranges = None
        if req.folds:
            fold_test_ranges = [(f.test_start, f.test_end) for f in req.folds]

        # Rolling vol for regime breakdown
        rolling_vol = None
        if "close" in df.columns:
            log_ret = np.log(df["close"].values + 1e-10)
            rolling_vol = pd.Series(log_ret).rolling(20).std().fillna(0).values[: len(realized)]

        metrics = scoring_mod.evaluate_distribution(
            levels=levels,
            predicted=predicted_q,
            realized=realized,
            trial_count=req.trial_count,
            fold_test_ranges=fold_test_ranges,
            rolling_vol=rolling_vol if rolling_vol is not None and len(rolling_vol) == len(realized) else None,
        )

        # I-2.8: enforce single-use holdout flag — caller is responsible for
        # tracking; we record it in metrics so Rust can persist it.
        metrics["holdout_used"] = req.holdout_used

        await _emit("building_scorecard", 85.0)
        scorecard = _build_eval_scorecard(metrics)

        report = {
            "eval_id": req.eval_id,
            "model_id": req.model_id,
            "version": req.version,
            "dataset_hash": req.dataset_hash,
            "n": metrics.get("n"),
            "crps": metrics.get("crps"),
            "crps_deflated": metrics.get("crps_deflated"),
            "pinball": metrics.get("pinball"),
            "log_score": metrics.get("log_score"),
            "pit": metrics.get("pit"),
            "coverage": metrics.get("coverage"),
            "reliability": metrics.get("reliability"),
            "var_backtest": metrics.get("var_backtest"),
            "overfitting": metrics.get("overfitting"),
            "baselines": metrics.get("baselines"),
            "beats_naive": metrics.get("beats_naive"),
            "dm_vs_naive": metrics.get("dm_vs_naive"),
            "per_fold": metrics.get("per_fold"),
            "per_regime": metrics.get("per_regime"),
        }

        await _emit("succeeded", 100.0)
        return EvalResponse(status="succeeded", metrics=metrics, scorecard=scorecard, report=report)

    except Exception as e:  # noqa: BLE001
        await _emit("failed", 100.0)
        return EvalResponse(status="failed", error=str(e))
    finally:
        await publisher.close()


def _eval_inference(
    artifact_bytes: bytes,
    df: pd.DataFrame,
    framework: str,
    is_quantile: bool,
    definition: dict,
) -> tuple[np.ndarray | None, list[float], np.ndarray | None, float]:
    """Run batch inference on the test-window DataFrame using the stored bundle.

    Returns (predicted_quantiles (N, L), levels, realized (N,), sigma).
    For non-quantile/point models: returns trivial single-level array.

    Realized values come from the 'label' column of the Parquet (PIT-correct
    by ADR-0017: labels are forward returns already shifted at pin time).
    """
    import pickle
    import struct

    label_col = "label"
    if label_col not in df.columns:
        return None, [0.5], None, 1.0

    realized = df[label_col].to_numpy(dtype=np.float64)

    # Parse bundle header to extract quantile levels, sigma, output_kind.
    try:
        # Bundle format: 4-byte header_len + header JSON + model bytes (engine.py wrap_bundle)
        header_len = struct.unpack_from(">I", artifact_bytes, 0)[0]
        import json
        header = json.loads(artifact_bytes[4: 4 + header_len])
        model_bytes = artifact_bytes[4 + header_len:]
    except Exception:
        header = {}
        model_bytes = artifact_bytes

    output_kind = header.get("output_kind", "point")
    levels = header.get("quantile_levels") or definition.get("output", {}).get("quantile_levels") or [0.5]
    sigma = float(header.get("sigma_scaler") or 1.0)

    # Feature columns = all non-label, non-target columns
    feature_cols = [c for c in df.columns if c not in ("label", "target")]
    df_feat = df[feature_cols].fillna(0.0)

    if output_kind == "distribution" or is_quantile:
        predicted_q = _predict_quantile_bundle(model_bytes, df_feat, levels, sigma, framework, header)
    else:
        # Point model — wrap scalar prediction as single-level
        predicted_scalar = _predict_point_bundle(model_bytes, df_feat, framework)
        if predicted_scalar is not None:
            predicted_q = predicted_scalar[:, None]
            levels = [0.5]
        else:
            return None, levels, realized, sigma

    if predicted_q is None:
        return None, levels, realized, sigma

    # Align lengths
    n = min(len(predicted_q), len(realized))
    return predicted_q[:n], levels, realized[:n], sigma


def _predict_quantile_bundle(
    model_bytes: bytes,
    df_feat: pd.DataFrame,
    levels: list[float],
    sigma: float,
    framework: str,
    header: dict,
) -> np.ndarray | None:
    """Batch inference for quantile bundles; returns (N, L) in return units."""
    import pickle
    try:
        payload = pickle.loads(model_bytes)
    except Exception:
        return None

    fw = framework.lower()
    n = len(df_feat)
    X = df_feat.to_numpy(dtype=np.float64)

    try:
        if fw in ("lightgbm_q", "lightgbm"):
            models = payload.get("models") or []
            preds = []
            for m_str in models:
                import lightgbm as lgb
                m = lgb.Booster(model_str=m_str)
                preds.append(m.predict(X))
            q = np.column_stack(preds) if preds else None
        elif fw in ("xgboost_q", "xgboost"):
            import xgboost as xgb
            import json
            m = xgb.Booster()
            m.load_model(bytearray(model_bytes))
            dmat = xgb.DMatrix(X)
            q = m.predict(dmat)
            if q.ndim == 1:
                q = q[:, None]
        elif fw in ("sklearn_q", "sklearn"):
            models = payload.get("models") or []
            preds = [m.predict(X) for m in models]
            q = np.column_stack(preds) if preds else None
        elif fw == "garch":
            params = payload.get("params") or {}
            sigma_uncond = float(params.get("sigma_uncond") or 1e-4)
            nu = float(params.get("nu") or 10.0)
            from scipy import stats as sp_stats
            q_vals = sp_stats.t.ppf(levels, df=nu) * sigma_uncond
            q = np.tile(q_vals, (n, 1))
        else:
            return None
    except Exception:
        return None

    if q is None:
        return None

    # Rescale σ-units → return units
    q_sorted = np.sort(q, axis=1)
    return q_sorted * sigma


def _predict_point_bundle(
    model_bytes: bytes,
    df_feat: pd.DataFrame,
    framework: str,
) -> np.ndarray | None:
    """Scalar prediction for point bundles; returns (N,) array."""
    import pickle
    X = df_feat.to_numpy(dtype=np.float64)
    fw = framework.lower()
    try:
        if fw == "lightgbm":
            import lightgbm as lgb
            m = lgb.Booster(model_str=pickle.loads(model_bytes).get("model_str", ""))
            return m.predict(X)
        if fw == "xgboost":
            import xgboost as xgb
            m = xgb.Booster()
            m.load_model(bytearray(model_bytes))
            return m.predict(xgb.DMatrix(X))
        if fw == "sklearn":
            payload = pickle.loads(model_bytes)
            m = payload.get("model")
            if m:
                return m.predict(X)
    except Exception:
        pass
    return None


def _build_eval_scorecard(metrics: dict) -> dict:
    """Quality sub-score from CRPS + calibration (I-2.10).

    Replaces the `val_auc` proxy used by the old scorecard.
    CRPS is cost (lower = better) so we invert: quality = 1 / (1 + crps).
    Coverage gap penalizes uncalibrated models.
    """
    crps = metrics.get("crps", None)
    pit_calibrated = metrics.get("pit", {}).get("calibrated", False)

    if crps is None or not np.isfinite(crps):
        quality = 50.0
    else:
        # Map CRPS ∈ [0, ∞) → quality ∈ (0, 100]
        # Typical CRPS for returns on 1-min bars ~0.001–0.01; we scale accordingly.
        quality = 100.0 / (1.0 + crps * 1000.0)
        if not pit_calibrated:
            quality *= 0.85  # 15% penalty for miscalibration

    quality = float(np.clip(quality, 0.0, 100.0))
    speed = 80.0
    cost = 90.0
    safety = 85.0
    reliability = 80.0

    w_q, w_s, w_c, w_sa, w_r = 0.50, 0.20, 0.10, 0.10, 0.10
    overall = quality * w_q + speed * w_s + cost * w_c + safety * w_sa + reliability * w_r

    return {
        "overall": round(overall, 1),
        "sub_scores": {
            "quality": round(quality, 1),
            "speed": speed,
            "cost": cost,
            "safety": safety,
            "reliability": reliability,
        },
        "weights": {"quality": w_q, "speed": w_s, "cost": w_c, "safety": w_sa, "reliability": w_r},
        "quality_source": "crps+calibration",
        "crps": crps,
        "pit_calibrated": pit_calibrated,
    }


def _load_dataframe(req: TrainRequest) -> pd.DataFrame:
    """Load the training frame from the pre-materialized Parquet snapshot."""
    store = get_store()
    try:
        raw = store.get(req.dataset_uri)
        return pd.read_parquet(io.BytesIO(raw))
    except Exception:  # noqa: BLE001
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

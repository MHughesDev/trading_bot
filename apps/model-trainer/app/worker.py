import asyncio
import io
import os

import numpy as np
import pandas as pd

from .schemas import (
    TrainRequest,
    TrainResponse,
    EvalRequest,
    EvalResponse,
    PostHocRequest,
    PostHocResponse,
    PostHocStep,
)
from . import failures
from . import pit
from . import posthoc as posthoc_mod
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
    # N-2.1: the family registry owns routing. `definition["family"]` (set by the
    # platform) wins; otherwise the legacy framework alias map reaches the same
    # adapters as before. An unknown pair raises `UnknownFamily` (a ValueError).
    from .families import registry as _registry

    return _registry.route(framework, model_kind, definition)


def _merge_declaration_family(req) -> None:
    """Set N: the platform's top-level `declaration`/`family` become part of the definition,
    so registry lookup and sequence windowing read them from one place."""
    if getattr(req, "declaration", None) is not None:
        req.definition["declaration"] = req.declaration
    if getattr(req, "family", None) is not None:
        req.definition["family"] = req.family


async def run_training(req: TrainRequest) -> TrainResponse:
    _merge_declaration_family(req)
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

        # Set N (N-1.7, N-1.5): subsample to the declared sampling events, then run the
        # leakage harness on what will actually be trained on. A failing dataset raises
        # `LeakageDetected` (terminal `leakage_detected`) before any model is fitted.
        from . import leakage as _leakage, sampling as _sampling

        df, req.folds, _sampling_info = _sampling.apply_to_request(req.definition, df, req.folds)
        _leak_report = _leakage.check_request(
            req.definition, df, req.folds, data_snapshot=req.dataset_hash
        )

        train_fn = _route(req.framework, req.model_kind, req.definition)

        # Inject fold-aware prepare when Rust supplied folds (I-0.10).
        if req.folds:
            req.definition["_wf_fold"] = req.folds[0].model_dump()

        # Mid-run checkpoints, for the frameworks AT-61 showed resume bit-identical.
        # `_resume` is the platform's `plan_resume` verdict (a manifest it already
        # approved); this process executes it and never second-guesses it.
        from . import checkpoint as _checkpoint

        ckpt_cfg = _checkpoint.request_config(
            framework=req.framework,
            output_prefix=req.output_prefix,
            dataset_hash=req.dataset_hash,
            resume=req.resume or req.definition.pop("_resume", None),
        )
        if ckpt_cfg is not None:
            req.definition["_checkpoint"] = ckpt_cfg

        artifact_bytes, metrics = await loop.run_in_executor(
            None, lambda: train_fn(req.definition, df, emit)
        )
        # How far this model's data reached: a backtest may ask it only for
        # decisions after this plus the embargo (Set U U-5.4).
        through = pit.trained_through_ns(df, req.folds)
        if through is not None:
            metrics = {**(metrics or {}), "trained_through_ns": through}
        metrics = {**(metrics or {}), "leakage": _leak_report,
                   **({"sampling": _sampling_info} if _sampling_info else {})}

        key = f"{req.output_prefix.rstrip('/')}/model.bin"
        key = key.replace("file://", "")
        uri, sha256, size = store.put(key, artifact_bytes)

        # One model per later walk-forward fold, each with its own training
        # end, so a backtest has a model for every fold's window (Set U U-5.8).
        from . import fold_models

        fold_artifacts = await loop.run_in_executor(
            None,
            lambda: fold_models.train_fold_models(
                req.definition, df, req.folds, train_fn, store.put, req.output_prefix, emit
            ),
        )

        resp = TrainResponse(
            status="succeeded",
            artifact_uri=uri,
            sha256=sha256,
            size_bytes=size,
            fold_artifacts=fold_artifacts or None,
            metrics=metrics,
            framework_version=str(metrics.get("framework_version")) if metrics else None,
        )
        await publisher.publish(subject, {
            "run_id": req.run_id, "phase": "succeeded", "progress": 100.0,
            "metric": metrics or {},
        })
    except Exception as e:  # noqa: BLE001
        classified = failures.classify_detailed(e)
        terminal = classified.reason
        resp = TrainResponse(
            status="failed",
            error=str(e),
            terminal=terminal,
            terminal_basis=classified.basis,
            exception_type=classified.exception_type,
        )
        await publisher.publish(subject, {
            "run_id": req.run_id, "phase": "failed", "progress": 100.0,
            "metric": {
                "error": str(e),
                "terminal": terminal,
                "terminal_basis": classified.basis,
            },
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
    _merge_declaration_family(req)
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
            return EvalResponse(status="failed", error=f"artifact load failed: {e}", terminal=failures.DATA_ERROR)

        # Load test-window dataset (Parquet from Phase 0 materialization)
        await _emit("loading_dataset", 15.0)
        try:
            raw = store.get(req.dataset_uri)
            df = pd.read_parquet(io.BytesIO(raw))
        except Exception as e:  # noqa: BLE001
            return EvalResponse(status="failed", error=f"dataset load failed: {e}", terminal=failures.DATA_ERROR)

        await _emit("running_inference", 30.0)

        # Determine framework from definition
        definition = req.definition
        framework = (definition.get("framework") or "").lower()
        output = definition.get("output") or {}
        is_quantile = bool(output.get("quantile_levels"))
        if isinstance(definition.get("family"), dict) and definition["family"].get("family") == "vol":
            framework = "garch"

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
            return EvalResponse(
                status="failed",
                error="no predictions or realized values produced",
                terminal=failures.DATA_ERROR,
            )

        # Vol models have a warm-up prefix with no forecast: drop it, shift fold ranges.
        _off = 0
        _fin = np.all(np.isfinite(predicted_q), axis=1)
        if not _fin.all():
            _off = int(np.argmax(_fin)) if _fin.any() else len(_fin)
            if not _fin[_off:].all():
                return EvalResponse(status="failed", error="non-finite predictions after warm-up",
                                    terminal=failures.NAN_DIVERGENCE)
            predicted_q, realized = predicted_q[_off:], realized[_off:]
            if np.ndim(sigma) > 0:
                sigma = np.asarray(sigma)[_off:]

        await _emit("scoring", 60.0)

        # Fold test ranges for per-fold breakdown
        fold_test_ranges = None
        if req.folds:
            fold_test_ranges = [(max(f.test_start - _off, 0), max(f.test_end - _off, 0)) for f in req.folds]

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

        # Set N (N-2.2): family baselines on the same folds, and the aligned series block.
        _attach_baselines_and_series(
            metrics, req, definition, df, predicted_q, levels, realized, sigma, _off, framework,
        )

        # Set N (N-3.3): purged, clustered permutation importance inputs for the platform.
        _attach_importance(metrics, req, df, artifact_bytes, framework, is_quantile, definition,
                           levels, _off)

        await _emit("building_scorecard", 85.0)
        scorecard = _build_eval_scorecard(metrics)
        scorecard["family_baselines"] = metrics.get("family_baselines")
        scorecard["fold_ids"] = metrics.get("fold_ids")

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
            "family_baselines": metrics.get("family_baselines"),
            "beats_naive": metrics.get("beats_naive"),
            "dm_vs_naive": metrics.get("dm_vs_naive"),
            "per_fold": metrics.get("per_fold"),
            "per_regime": metrics.get("per_regime"),
        }

        await _emit("succeeded", 100.0)
        return EvalResponse(status="succeeded", metrics=metrics, scorecard=scorecard, report=report)

    except Exception as e:  # noqa: BLE001
        await _emit("failed", 100.0)
        return EvalResponse(status="failed", error=str(e), terminal=failures.classify(e))
    finally:
        await publisher.close()


def _attach_importance(metrics, req, df, artifact_bytes, framework, is_quantile, definition,
                       levels, off):
    """`metrics["importance_inputs"]` (shape in app/importance.py), or a stated reason.

    Never chooses test rows: they are the platform's folds. Skipped, with the reason recorded,
    when there are no folds, the model has a warm-up prefix, or there are too few features.
    """
    from . import importance as imp

    try:
        if not req.folds or off or framework == "garch":
            metrics["importance_inputs"] = {"skipped": "no folds, or a model with a warm-up prefix"}
            return
        feature_cols = [c for c in df.columns if c not in ("label", "target")]
        if len(feature_cols) < 2:
            metrics["importance_inputs"] = {"skipped": "fewer than two features"}
            return
        x = df[feature_cols].fillna(0.0).to_numpy(dtype=np.float64)
        y = df["label"].to_numpy(dtype=np.float64)
        median_col = levels.index(0.5) if 0.5 in levels else len(levels) // 2

        def predict(xs):
            frame = pd.DataFrame(xs, columns=feature_cols)
            frame["label"] = 0.0
            q, _, _, _ = _eval_inference(artifact_bytes, frame, framework, is_quantile, definition)
            if q is None:
                raise ValueError("the model produced no predictions for importance")
            return q[:, median_col if q.shape[1] > 1 else 0]

        clusters = imp.correlation_clusters(x)
        ranges = [(f.test_start, f.test_end) for f in req.folds]
        out = imp.permutation_increases(predict, x, y, ranges, clusters)
        out["features"] = feature_cols
        metrics["importance_inputs"] = out
    except Exception as e:  # noqa: BLE001 - recorded, never silent
        metrics["importance_inputs"] = {"skipped": f"importance failed: {e}"}


def _attach_baselines_and_series(metrics, req, definition, df, predicted_q, levels, realized,
                                 sigma, off, framework):
    """`metrics["family_baselines"]`, `["fold_ids"]`, `["candidate"]`, `["series"]`.
    Shapes are documented in app/families/baseline.py. Refuses a candidate without baselines."""
    from .families import baseline as bl

    if not req.run_baselines:
        raise bl.BaselineMissing("a candidate is never evaluated without its baselines (run_baselines=false)")
    # Keep the old naive/zero-shot dict under `baselines` untouched.
    d = df.iloc[off:].reset_index(drop=True)
    folds = None
    if req.folds:
        shifted = []
        for f in req.folds:
            m = f.model_dump()
            for k in ("train_start", "train_end", "cal_start", "cal_end", "test_start", "test_end"):
                m[k] = max(m[k] - off, 0)
            shifted.append(m)
        folds = shifted
    rows, fold_ids = bl.evaluate_baselines(definition, d, folds)
    metrics["family_baselines"] = rows
    metrics["fold_ids"] = fold_ids
    lv = np.asarray(levels, float)
    if predicted_q.shape[1] == 1:
        point = predicted_q[:, 0]
    else:
        point = predicted_q[:, int(np.argmin(np.abs(lv - 0.5)))]
    kind_t = bl.target_kind(definition)
    is_vol = kind_t == "realized_vol" or framework == "garch"
    sig = None
    if np.ndim(sigma) > 0:
        sig = np.asarray(sigma, float)
    elif predicted_q.shape[1] > 1 and lv.min() <= 0.05 + 1e-9 and lv.max() >= 0.95 - 1e-9:
        lo = predicted_q[:, int(np.argmin(np.abs(lv - 0.05)))]
        hi = predicted_q[:, int(np.argmin(np.abs(lv - 0.95)))]
        sig = (hi - lo) / (2 * 1.6449)
    cand = bl.score_candidate(definition, d.assign(label=realized), point, folds)
    metrics["candidate"] = cand
    metrics["series"] = {
        "target_kind": "vol" if is_vol else "return",
        "realized": bl.series_list(realized),
        "point": bl.series_list(point),
        "sigma": None if sig is None else bl.series_list(sig),
    }


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
        import json
        from . import engine as _eng
        if artifact_bytes[:8] == _eng.MAGIC:
            # tb-bundle-1: MAGIC(8) | header_len(u32 LE) | header json | inner
            header_len = struct.unpack_from("<I", artifact_bytes, 8)[0]
            header = json.loads(artifact_bytes[12: 12 + header_len])
            model_bytes = artifact_bytes[12 + header_len:]
        else:
            header_len = struct.unpack_from(">I", artifact_bytes, 0)[0]
            header = json.loads(artifact_bytes[4: 4 + header_len])
            model_bytes = artifact_bytes[4 + header_len:]
    except Exception:
        header = {}
        model_bytes = artifact_bytes

    if framework == "garch" or header.get("framework") == "garch":
        return _eval_vol(model_bytes, df, levels_hint=header.get("quantile_levels"),
                         definition=definition, realized=realized)

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


def _eval_vol(model_bytes, df, *, levels_hint, definition, realized):
    """Vol-family batch inference: per-row one-step conditional sigma (no unconditional
    fallback). Leading warm-up rows are NaN; the caller drops that prefix."""
    from .families import vol

    state = vol.state_from_bundle_inner(model_bytes)
    levels = list(levels_hint or [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95])
    r, xr, _src = vol.returns_from_frame(definition, df, state.get("returns_source"))
    sig = vol.sigma_path(state, r, xr if state["kind"] == "realized_garch" else None)
    q = vol.quantiles(state, np.nan_to_num(sig, nan=1.0), levels)
    q[~np.isfinite(sig), :] = np.nan
    return np.sort(q, axis=1), levels, realized, sig


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
            # N-4.2: conditional sigma lives in `_eval_vol`; there is no unconditional path.
            return None
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


async def run_posthoc(req: PostHocRequest) -> PostHocResponse:
    """The post-hoc pipeline as a job (SPEC 11.4, checklist 2.11).

    One call, four steps, one order. The response carries every step including
    the ones that did not apply, so the sequence stays auditable from the
    artifact alone.
    """
    try:
        report = posthoc_mod.run_posthoc(
            framework=req.framework,
            costs=posthoc_mod.CostMatrix(
                true_positive=req.costs.true_positive,
                false_positive=req.costs.false_positive,
                true_negative=req.costs.true_negative,
                false_negative=req.costs.false_negative,
            ),
            oos_predictions=np.asarray(req.oos_predictions, dtype=np.float64),
            oos_realized=np.asarray(req.oos_realized, dtype=np.float64),
            cal_scores=np.asarray(req.cal_scores, dtype=np.float64),
            cal_outcomes=np.asarray(req.cal_outcomes, dtype=np.float64),
            calibrator=req.calibrator,
        )
    except Exception as e:  # noqa: BLE001
        return PostHocResponse(status="failed", error=str(e), terminal=failures.classify(e))

    return PostHocResponse(
        status="succeeded",
        steps=[
            PostHocStep(step=s.step, applied=s.applied, detail=s.detail, skipped=s.skipped)
            for s in report.steps
        ],
        weights=None if report.weights is None else [float(w) for w in report.weights],
        calibrator=report.calibrator,
        calibration_params=report.calibration_params,
        threshold=report.threshold,
    )

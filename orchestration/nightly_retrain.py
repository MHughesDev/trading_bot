"""
Nightly retrain flow — Prefect + MLflow (optional).

Spec: no auto model promotion; manual gate after evaluation.
"""

from __future__ import annotations

import logging
import tempfile
from pathlib import Path
from typing import Any

import numpy as np

from models.forecast.tft_forecast import TemporalFusionForecaster
from models.regime.hmm_regime import GaussianHMMRegimeModel
from models.registry.mlflow_registry import MLflowModelRegistry

logger = logging.getLogger(__name__)


def _synthetic_training_data(
    *,
    n_samples: int = 200,
    n_features: int = 4,
    seed: int = 42,
) -> tuple[np.ndarray, np.ndarray]:
    rng = np.random.default_rng(seed)
    X = rng.normal(size=(n_samples, n_features)).astype(np.float64)
    y = rng.normal(size=(n_samples, 4)).astype(np.float64) * 1e-3
    return X, y


def run_train_evaluate_log(
    *,
    artifact_dir: str | Path | None = None,
    mlflow_tracking_uri: str | None = None,
) -> dict[str, Any]:
    """
    Train HMM + forecast surrogates on synthetic data, persist joblib artifacts, log params/metrics to MLflow.
    Replace `_synthetic_training_data` with historical bar features in production.
    """
    artifact_dir = Path(artifact_dir or tempfile.mkdtemp(prefix="nm-train-"))
    artifact_dir.mkdir(parents=True, exist_ok=True)
    regime_path = artifact_dir / "regime.joblib"
    forecast_path = artifact_dir / "forecast.joblib"

    X, _ = _synthetic_training_data()
    hmm = GaussianHMMRegimeModel(bootstrap_if_unfitted=False)
    hmm.fit(X)
    hmm.save(regime_path)

    Xf, yf = _synthetic_training_data(n_samples=300, n_features=32)
    fc = TemporalFusionForecaster(feature_dim=32, bootstrap_if_unfitted=False)
    fc.partial_fit_buffer(Xf, yf)
    fc.fit()
    fc.save(forecast_path)

    registry = MLflowModelRegistry(tracking_uri=mlflow_tracking_uri)
    registry.log_params_metrics(
        {"artifact_dir": str(artifact_dir), "regime_states": "4", "forecast": "ridge_surrogate"},
        {"train_samples_hmm": float(X.shape[0]), "train_samples_forecast": float(Xf.shape[0])},
    )

    logger.info(
        "train complete: regime=%s forecast=%s (manual promotion only per spec)",
        regime_path,
        forecast_path,
    )
    return {
        "regime_path": str(regime_path),
        "forecast_path": str(forecast_path),
        "artifact_dir": str(artifact_dir),
    }


def _prefect_flow_def():
    try:
        from prefect import flow
    except ImportError:
        return None

    @flow(name="nautilus-monster-nightly-retrain")
    def nightly_flow(
        artifact_dir: str | None = None,
        mlflow_tracking_uri: str | None = None,
    ) -> dict[str, Any]:
        return run_train_evaluate_log(
            artifact_dir=artifact_dir,
            mlflow_tracking_uri=mlflow_tracking_uri,
        )

    return nightly_flow


def nightly_flow_entrypoint() -> None:
    """CLI / deployment entry: `python -m orchestration.nightly_retrain`"""
    fn = _prefect_flow_def()
    if fn is None:
        logging.basicConfig(level=logging.INFO)
        logger.info("prefect not installed; running inline train (pip install nautilus-monster[orchestration])")
        run_train_evaluate_log()
        return
    fn()


if __name__ == "__main__":
    nightly_flow_entrypoint()

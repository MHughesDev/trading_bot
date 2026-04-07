"""MLflow model registry wrapper — optional dependency."""

from __future__ import annotations

import logging
from typing import Any

logger = logging.getLogger(__name__)


class MLflowModelRegistry:
    def __init__(self, tracking_uri: str | None = None, experiment: str = "nautilus-monster") -> None:
        self._tracking_uri = tracking_uri
        self._experiment = experiment
        self._mlflow: Any = None

    def _ensure(self) -> bool:
        if self._mlflow is not None:
            return True
        try:
            import mlflow

            if self._tracking_uri:
                mlflow.set_tracking_uri(self._tracking_uri)
            mlflow.set_experiment(self._experiment)
            self._mlflow = mlflow
            return True
        except ImportError:
            logger.warning("mlflow not installed; registry is no-op")
            return False

    def log_params_metrics(self, params: dict[str, Any], metrics: dict[str, float]) -> None:
        if not self._ensure():
            return
        with self._mlflow.start_run():
            self._mlflow.log_params(params)
            self._mlflow.log_metrics(metrics)

    def promote(self, _model_name: str, _version: int) -> None:
        """Master spec: no auto model promotion — human gate only; this method does not register stages."""
        logger.info(
            "promotion gated (spec): manual approval required for %s v%s — do not wire auto-stage in CI",
            _model_name,
            _version,
        )

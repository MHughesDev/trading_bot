from __future__ import annotations

from dataclasses import dataclass

try:
    import mlflow
except Exception:  # pragma: no cover - environment without mlflow still boots runtime.
    mlflow = None

from app.config.settings import MLflowSettings


@dataclass(slots=True)
class ModelRegistry:
    tracking_uri: str

    def __post_init__(self) -> None:
        if mlflow is not None:
            mlflow.set_tracking_uri(self.tracking_uri)

    def log_metric(
        self, run_name: str, metrics: dict[str, float], tags: dict[str, str] | None = None
    ) -> str:
        if mlflow is None:
            return "mlflow-disabled"
        with mlflow.start_run(run_name=run_name) as run:
            if tags:
                mlflow.set_tags(tags)
            for key, value in metrics.items():
                mlflow.log_metric(key, value)
            return run.info.run_id

    def log_params(self, run_name: str, params: dict[str, str | int | float]) -> str:
        if mlflow is None:
            return "mlflow-disabled"
        with mlflow.start_run(run_name=run_name) as run:
            mlflow.log_params(params)
            return run.info.run_id


def build_registry(settings: MLflowSettings) -> ModelRegistry:
    return ModelRegistry(tracking_uri=settings.tracking_uri)

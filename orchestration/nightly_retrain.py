from __future__ import annotations

from datetime import UTC, datetime

from prefect import flow, task

from app.config.settings import load_settings
from models.registry.mlflow_registry import build_registry


@task
def fetch_training_data() -> dict[str, int]:
    # Placeholder for production data extract jobs from QuestDB/object storage.
    return {"rows": 10_000, "symbols": 3}


@task
def retrain_models(data_info: dict[str, int]) -> dict[str, float]:
    # Placeholder deterministic metrics.
    rows = float(data_info.get("rows", 0))
    coverage = min(rows / 10_000.0, 1.0)
    return {
        "regime_accuracy_proxy": 0.62 + 0.08 * coverage,
        "forecast_mae_proxy": 0.0042,
        "route_sharpe_proxy": 1.12,
    }


@task
def gate_promotion(metrics: dict[str, float]) -> bool:
    # Non-negotiable: no auto-promotion to production.
    # We only report whether a candidate is eligible for manual review.
    return metrics["route_sharpe_proxy"] >= 1.0 and metrics["regime_accuracy_proxy"] >= 0.6


@flow(name="nautilusmonster-nightly-retrain")
def nightly_retrain_flow() -> dict[str, str | float | bool]:
    settings = load_settings()
    registry = build_registry(settings.storage.mlflow)

    data_info = fetch_training_data()
    metrics = retrain_models(data_info)
    eligible = gate_promotion(metrics)
    run_id = registry.log_metric(
        run_name=f"nightly-{datetime.now(UTC).strftime('%Y%m%d-%H%M%S')}",
        metrics=metrics,
        tags={"eligible_for_manual_promotion": str(eligible).lower()},
    )

    return {"run_id": run_id, "eligible_for_manual_promotion": eligible, **metrics}


if __name__ == "__main__":
    result = nightly_retrain_flow()
    print(result)

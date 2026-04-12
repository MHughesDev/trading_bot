"""
Initial forecaster artifact for per-asset init (FB-AP-010).

Runs :func:`forecaster_model.training.distill_mlp.train_distilled_mlp_forecaster` into
``<init_run_dir>/forecaster/`` — **synthetic teacher** distillation (same as CLI
``tb-train-forecaster-distill``), scoped per symbol/job so manifests can point at
asset-specific paths. Real **bars→tensor** training is a follow-up; this closes the
pipeline wiring and directory contract.

Requires optional extra ``[models_torch]`` (``torch``).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from app.config.settings import AppSettings


def run_init_forecaster_distill(
    *,
    run_dir: Path,
    settings: AppSettings,
    symbol: str,
) -> dict[str, Any]:
    """
    Write ``forecaster_torch.pt`` and sidecar JSON under ``run_dir/forecaster/``.

    Raises:
        ImportError: if ``torch`` is not installed.
    """
    from forecaster_model.training.distill_mlp import train_distilled_mlp_forecaster

    forecaster_dir = run_dir / "forecaster"
    epochs = max(1, int(settings.asset_init_forecaster_distill_epochs))
    meta = train_distilled_mlp_forecaster(
        artifact_dir=forecaster_dir,
        epochs=epochs,
        device=settings.models_torch_device,
    )
    pt_path = Path(str(meta.get("weights", forecaster_dir / "forecaster_torch.pt")))
    out: dict[str, Any] = {
        "symbol": symbol.strip(),
        "trainer": "train_distilled_mlp_forecaster",
        "methodology": "distill_mlp_synthetic_teacher",
        "epochs": epochs,
        "forecaster_dir": str(forecaster_dir.resolve()),
        "forecaster_torch": str(pt_path.resolve()) if pt_path.exists() else str(pt_path),
        "forecaster_train_meta": str((forecaster_dir / "forecaster_train_meta.json").resolve()),
    }
    return out


def init_forecaster_detail_payload(payload: dict[str, Any]) -> dict[str, Any]:
    """JSON-safe subset for init job step detail."""
    return {
        "symbol": payload.get("symbol"),
        "trainer": payload.get("trainer"),
        "methodology": payload.get("methodology"),
        "epochs": payload.get("epochs"),
        "forecaster_torch": payload.get("forecaster_torch"),
        "forecaster_dir": payload.get("forecaster_dir"),
    }

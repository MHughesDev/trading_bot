"""Training utilities: walk-forward splits, metrics, checkpoints (FB-FR-P1 scaffolding)."""

from forecaster_model.training.checkpoint import load_json_checkpoint, save_json_checkpoint
from forecaster_model.training.device import resolve_torch_device
from forecaster_model.training.metrics import mean_pinball_loss, pinball_loss
from forecaster_model.training.torch_trainer import train_forecaster_stub, train_forecaster_torch
from forecaster_model.training.walkforward import WalkForwardConfig, walk_forward_indices

__all__ = [
    "WalkForwardConfig",
    "walk_forward_indices",
    "pinball_loss",
    "mean_pinball_loss",
    "save_json_checkpoint",
    "load_json_checkpoint",
    "resolve_torch_device",
    "train_forecaster_stub",
    "train_forecaster_torch",
]

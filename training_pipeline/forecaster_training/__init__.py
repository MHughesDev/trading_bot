"""Training utilities: walk-forward splits, metrics, checkpoints (FB-FR-P1 scaffolding)."""

from training_pipeline.forecaster_training.checkpoint import load_json_checkpoint, save_json_checkpoint
from training_pipeline.forecaster_training.device import resolve_torch_device
from training_pipeline.forecaster_training.metrics import mean_pinball_loss, pinball_loss
from training_pipeline.forecaster_training.distill_mlp import train_distilled_mlp_forecaster
from training_pipeline.forecaster_training.torch_trainer import train_forecaster_stub, train_forecaster_torch
from training_pipeline.forecaster_training.walk_forward_torch import describe_walk_forward_folds
from training_pipeline.forecaster_training.walkforward import WalkForwardConfig, walk_forward_indices

__all__ = [
    "WalkForwardConfig",
    "walk_forward_indices",
    "describe_walk_forward_folds",
    "pinball_loss",
    "mean_pinball_loss",
    "save_json_checkpoint",
    "load_json_checkpoint",
    "resolve_torch_device",
    "train_forecaster_stub",
    "train_forecaster_torch",
    "train_distilled_mlp_forecaster",
]

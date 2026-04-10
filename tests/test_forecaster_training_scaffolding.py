"""forecaster_model.training scaffolding (FB-FR-P1)."""

from __future__ import annotations

import tempfile
from pathlib import Path

import numpy as np

from forecaster_model.training.checkpoint import load_json_checkpoint, save_json_checkpoint
from forecaster_model.training.metrics import mean_pinball_loss, pinball_loss
from forecaster_model.training.walkforward import WalkForwardConfig, walk_forward_indices


def test_pinball_loss() -> None:
    assert pinball_loss(1.0, 0.5, 0.5) >= 0


def test_mean_pinball() -> None:
    y = np.array([0.0, 1.0, -0.5])
    pred = np.zeros((3, 3))
    m = mean_pinball_loss(y, pred, (0.1, 0.5, 0.9))
    assert m >= 0


def test_walk_forward_indices() -> None:
    cfg = WalkForwardConfig(train_len=10, val_len=5, step=5)
    splits = walk_forward_indices(30, cfg)
    assert len(splits) >= 1
    tr, va = splits[0]
    assert tr.stop <= va.start


def test_json_checkpoint_roundtrip() -> None:
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "ckpt.json"
        save_json_checkpoint(p, {"epoch": 1, "loss": 0.1})
        d = load_json_checkpoint(p)
        assert d["epoch"] == 1

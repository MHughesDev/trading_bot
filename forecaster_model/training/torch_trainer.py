"""
PyTorch forecaster training stack (FB-FR-PG2 / FB-FR-P0).

Optional dependency: `torch`. When unavailable, `train_forecaster_stub` documents the contract
and writes a JSON checkpoint only (no neural weights).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from forecaster_model.training.checkpoint import save_json_checkpoint


def train_forecaster_stub(
    *,
    artifact_dir: str | Path,
    epochs: int = 1,
    patience: int = 5,
) -> dict[str, Any]:
    """
    Placeholder training loop: records hyperparameters and early-stopping config for CI/docs.

    When `torch` is installed, `train_forecaster_torch` runs a minimal loop (future extension).
    """
    artifact_dir = Path(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    meta = {
        "trainer": "stub",
        "epochs": epochs,
        "early_stopping_patience": patience,
        "note": "Install torch and extend train_forecaster_torch for full FB-FR-P0",
    }
    save_json_checkpoint(artifact_dir / "forecaster_train_meta.json", meta)
    return meta


def train_forecaster_torch(
    *,
    artifact_dir: str | Path,
    epochs: int = 2,
    learning_rate: float = 1e-3,
) -> dict[str, Any]:
    """Minimal PyTorch train step when torch is installed; else raises ImportError."""
    try:
        import torch
        import torch.nn as nn
    except ImportError as e:
        raise ImportError("Install nautilus-monster[models_torch] for PyTorch forecaster training") from e

    artifact_dir = Path(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    # Toy linear model: proves train/eval/save path without full VSN/xLSTM graph
    model = nn.Linear(8, 4)
    opt = torch.optim.Adam(model.parameters(), lr=learning_rate)
    x = torch.randn(16, 8)
    y = torch.randn(16, 4)
    for _ in range(epochs):
        opt.zero_grad()
        loss = ((model(x) - y) ** 2).mean()
        loss.backward()
        opt.step()
    path = artifact_dir / "forecaster_toy.pt"
    torch.save({"state_dict": model.state_dict(), "epochs": epochs}, path)
    meta = {"trainer": "torch_toy_linear", "weights": str(path), "loss": float(loss.detach())}
    save_json_checkpoint(artifact_dir / "forecaster_train_meta.json", meta)
    return meta

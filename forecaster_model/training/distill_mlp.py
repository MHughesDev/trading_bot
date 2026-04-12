"""
Distill the NumPy reference forecaster into `build_torch_forecaster` (MLP) weights.

Closes the FB-FR-P0 train/serve loop for the **shipped** hot-path architecture (MLP quantile head),
without requiring the full VSN/xLSTM PyTorch graph in training.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from forecaster_model.config import ForecasterConfig
from forecaster_model.models.numpy_reference import forward_numpy_reference
from forecaster_model.models.torch_forecaster_net import build_torch_forecaster, _obs_feature_dim
from forecaster_model.training.checkpoint import save_json_checkpoint
from forecaster_model.training.device import resolve_torch_device


def _random_inputs(
    cfg: ForecasterConfig,
    rng: Any,
    batch_size: int,
) -> tuple[Any, Any, Any]:
    L = cfg.history_length
    f_obs = _obs_feature_dim(cfg)
    H = cfg.forecast_horizon
    f_known = 6
    rdim = cfg.num_regime_dims
    x_obs = rng.standard_normal(size=(batch_size, L, f_obs))
    x_known = rng.standard_normal(size=(batch_size, H, f_known))
    r = rng.random(size=(batch_size, rdim))
    r = r / r.sum(axis=1, keepdims=True)
    return x_obs, x_known, r


def train_distilled_mlp_forecaster(
    *,
    artifact_dir: str | Path,
    cfg: ForecasterConfig | None = None,
    teacher_seed: int = 42,
    train_seed: int = 123,
    epochs: int = 8,
    batch_size: int = 16,
    steps_per_epoch: int = 16,
    learning_rate: float = 1e-3,
    device: str | None = "auto",
) -> dict[str, Any]:
    """
    Synthetic distillation: teacher = `forward_numpy_reference`, student = MLP from `build_torch_forecaster`.

    Writes ``forecaster_torch.pt`` (torch.save dict) + ``forecaster_bundle_manifest.json`` + ``forecaster_train_meta.json``.
    """
    try:
        import numpy as np
        import torch
        import torch.nn as nn
    except ImportError as e:
        raise ImportError("Install trading-bot[models_torch] for distilled MLP training") from e

    cfg = cfg or ForecasterConfig()
    artifact_dir = Path(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)

    dev = torch.device(resolve_torch_device(device))
    rng = np.random.default_rng(train_seed)
    model = build_torch_forecaster(cfg).to(dev)
    opt = torch.optim.Adam(model.parameters(), lr=learning_rate)
    loss_fn = nn.MSELoss()

    for _ in range(epochs):
        for _ in range(steps_per_epoch):
            xo, xk, rr = _random_inputs(cfg, rng, batch_size)
            targets: list[Any] = []
            for b in range(batch_size):
                y_t, _ = forward_numpy_reference(
                    xo[b], xk[b], rr[b], cfg, seed=teacher_seed + b
                )
                targets.append(y_t)
            y_stack = np.stack(targets, axis=0)
            t_t = torch.from_numpy(y_stack.astype(np.float32)).to(dev)

            xo_t = torch.from_numpy(xo.astype(np.float32)).to(dev)
            xk_t = torch.from_numpy(xk.astype(np.float32)).to(dev)
            rr_t = torch.from_numpy(rr.astype(np.float32)).to(dev)

            opt.zero_grad()
            pred = model(xo_t, xk_t, rr_t)
            loss = loss_fn(pred, t_t)
            loss.backward()
            opt.step()

    pt_path = artifact_dir / "forecaster_torch.pt"
    bundle = {
        "state_dict": model.state_dict(),
        "forecaster_config": cfg,
        "device": str(dev),
        "trainer": "distill_mlp_from_numpy_reference",
        "teacher_seed": teacher_seed,
        "train_seed": train_seed,
        "epochs": epochs,
        "batch_size": batch_size,
        "steps_per_epoch": steps_per_epoch,
    }
    torch.save(bundle, pt_path)

    manifest = {
        "schema": "tb_forecaster_torch_bundle_v1",
        "weights_file": pt_path.name,
        "forecaster_config": {
            "history_length": cfg.history_length,
            "forecast_horizon": cfg.forecast_horizon,
            "quantiles": list(cfg.quantiles),
            "feature_windows": list(cfg.feature_windows),
            "num_regime_dims": cfg.num_regime_dims,
        },
        "methodology": "distill_mlp_synthetic_teacher",
    }
    mpath = artifact_dir / "forecaster_bundle_manifest.json"
    mpath.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    meta = {
        "trainer": "distill_mlp_forecaster",
        "weights": str(pt_path),
        "manifest": str(mpath),
        "final_loss": float(loss.detach().cpu()),
        "device": str(dev),
    }
    save_json_checkpoint(artifact_dir / "forecaster_train_meta.json", meta)
    return meta


def main() -> None:
    import argparse

    p = argparse.ArgumentParser(description="Distill NumPy reference → PyTorch MLP forecaster")
    p.add_argument("--artifact-dir", type=Path, default=Path("models/artifacts_training/torch_distill"))
    p.add_argument("--epochs", type=int, default=8)
    p.add_argument("--device", type=str, default="auto")
    args = p.parse_args()
    out = train_distilled_mlp_forecaster(artifact_dir=args.artifact_dir, epochs=args.epochs, device=args.device)
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()

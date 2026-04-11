"""Load and run PyTorch forecaster checkpoints (optional `[models_torch]`)."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np

from forecaster_model.config import ForecasterConfig
from forecaster_model.models.torch_forecaster_net import build_torch_forecaster


def load_torch_forecaster_checkpoint(
    path: str | Path,
    *,
    cfg: ForecasterConfig | None = None,
) -> tuple[Any, Any, ForecasterConfig]:
    """Load `.pt` checkpoint: expects dict with `state_dict` and optional `cfg` / `forecaster_config`."""
    try:
        import torch
    except ImportError as e:
        raise ImportError("Install nautilus-monster[models_torch] for PyTorch forecaster loading") from e

    path = Path(path)
    raw: Any = torch.load(path, map_location="cpu", weights_only=False)
    c = cfg or ForecasterConfig()
    if isinstance(raw, dict):
        if "forecaster_config" in raw and isinstance(raw["forecaster_config"], ForecasterConfig):
            c = raw["forecaster_config"]
        elif "cfg" in raw and isinstance(raw["cfg"], ForecasterConfig):
            c = raw["cfg"]
        state = raw.get("state_dict", raw.get("model_state_dict"))
        if state is None and "net.0.weight" in raw:
            state = raw
    else:
        state = raw

    model = build_torch_forecaster(c)
    if state is not None:
        model.load_state_dict(state, strict=False)
    dev_s = "cpu"
    if isinstance(raw, dict):
        dev_s = str(raw.get("device", "cpu"))
    dev = torch.device(dev_s if dev_s.startswith("cuda") or dev_s == "cpu" else "cpu")
    model = model.to(dev)
    model.eval()
    return model, dev, c


def forward_torch_quantiles(
    x_obs: np.ndarray,
    x_known: np.ndarray,
    r_cur: np.ndarray,
    *,
    model: Any,
    device: Any,
) -> np.ndarray:
    """Run single-window forward; returns [H, Qn] float64."""
    try:
        import torch
    except ImportError as e:
        raise ImportError("torch required for forward_torch_quantiles") from e

    model.eval()
    with torch.no_grad():
        xo = torch.from_numpy(np.asarray(x_obs, dtype=np.float64)).unsqueeze(0).to(device)
        xk = torch.from_numpy(np.asarray(x_known, dtype=np.float64)).unsqueeze(0).to(device)
        rr = torch.from_numpy(np.asarray(r_cur, dtype=np.float64).reshape(1, -1)).to(device)
        out = model(xo, xk, rr)
        return out.squeeze(0).detach().cpu().numpy().astype(np.float64)

"""
Backward-compatible forward helper — delegates to `ForecasterModel` (spec §20).

Implements the canonical pipeline: VSN → Latent CNN → Multi-Resolution xLSTM → Fusion → Quantile Decoder.
"""

from __future__ import annotations

import numpy as np

from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_model import ForecasterModel


def forward_numpy_reference(
    x_obs: np.ndarray,
    x_known: np.ndarray,
    r_cur: np.ndarray,
    cfg: ForecasterConfig | None = None,
    *,
    seed: int = 42,
) -> tuple[np.ndarray, dict]:
    model = ForecasterModel(cfg=cfg, seed=seed)
    out = model.forward(x_obs, x_known, r_cur)
    y_hat = out["y_hat_q"]
    assert isinstance(y_hat, np.ndarray)
    diag = {
        "vsn_gate_mean": float(out["gates"].mean()),
        "fusion_alpha": out["fusion_weights"].tolist()
        if hasattr(out["fusion_weights"], "tolist")
        else out["fusion_weights"],
        "branch_scales": list(out["branch_outputs"].keys()),
    }
    return y_hat, diag

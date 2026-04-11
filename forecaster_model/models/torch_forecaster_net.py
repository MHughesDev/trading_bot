"""Small MLP forecaster for distillation / optional hot-path inference (FB-FR-P0)."""

from __future__ import annotations

from typing import Any

from forecaster_model.config import ForecasterConfig


def _obs_feature_dim(cfg: ForecasterConfig) -> int:
    # Matches `build_observed_feature_matrix`: 5 base + 2 per window
    return 5 + 2 * len(cfg.feature_windows)


def build_torch_forecaster(cfg: ForecasterConfig | None = None) -> Any:
    """Flattened MLP: (x_obs, x_known, r_cur) -> [H, Qn] quantiles."""
    try:
        import torch
        import torch.nn as nn
    except ImportError as e:
        raise ImportError("torch required for build_torch_forecaster") from e

    cfg = cfg or ForecasterConfig()
    h = cfg.forecast_horizon
    qn = len(cfg.quantiles)
    L = cfg.history_length
    f_obs = _obs_feature_dim(cfg)
    f_known = 6
    rdim = cfg.num_regime_dims
    in_dim = L * f_obs + h * f_known + rdim

    class ForecasterTorchMLP(nn.Module):
        def __init__(self) -> None:
            super().__init__()
            self.net = nn.Sequential(
                nn.Linear(in_dim, 512),
                nn.ReLU(),
                nn.Linear(512, 256),
                nn.ReLU(),
                nn.Linear(256, h * qn),
            )

        def forward(self, x_obs: Any, x_known: Any, r_cur: Any) -> Any:
            b = int(x_obs.shape[0])
            z = torch.cat(
                [x_obs.reshape(b, -1), x_known.reshape(b, -1), r_cur],
                dim=1,
            )
            out = self.net(z).view(b, h, qn)
            return out

    return ForecasterTorchMLP()

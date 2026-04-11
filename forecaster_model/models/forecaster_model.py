"""
Top-level ForecasterModel composition (human forecaster spec §20.1).

Pipeline (spec §30, short form):

**VSN → Latent CNN → Multi-Resolution xLSTM → Regime-Conditioned Fusion → Quantile Decoder**
"""

from __future__ import annotations

import numpy as np

from forecaster_model.config import ForecasterConfig
from forecaster_model.models.decoder import forward_quantile_decoder
from forecaster_model.models.fusion import forward_regime_conditioned_fusion
from forecaster_model.models.latent_encoder import forward_latent_encoder
from forecaster_model.models.multi_resolution_xlstm import forward_multi_resolution_xlstm
from forecaster_model.models.vsn import forward_vsn


class ForecasterModel:
    """
    f_theta(X_obs, X_known, R_cur) -> Y_hat_q [H, Qn] (spec §4, §20).

    Optional x_static / regime_seq reserved for extensions (spec §20.1 forward signature).
    """

    def __init__(self, cfg: ForecasterConfig | None = None, *, seed: int = 42) -> None:
        self.cfg = cfg or ForecasterConfig()
        self._rng = np.random.default_rng(seed)

    def forward(
        self,
        x_obs: np.ndarray,
        x_known: np.ndarray,
        r_cur: np.ndarray,
        *,
        x_static: np.ndarray | None = None,
        regime_seq: np.ndarray | None = None,
    ) -> dict[str, object]:
        _ = x_static
        _ = regime_seq
        x_vsn, gates = forward_vsn(x_obs, self._rng)
        z_seq = forward_latent_encoder(x_vsn, self._rng)
        L = z_seq.shape[0]
        scales = (1, 4, 16) if L >= 16 else (1,)
        branches = forward_multi_resolution_xlstm(z_seq, scales, hidden_dim=32, rng=self._rng)
        fused, alpha = forward_regime_conditioned_fusion(branches, r_cur, self._rng)
        h_last = fused[-1]
        y_hat_q = forward_quantile_decoder(h_last, x_known, self.cfg.quantiles, self._rng)
        return {
            "y_hat_q": y_hat_q,
            "gates": gates,
            "branch_outputs": branches,
            "fusion_weights": alpha,
            "z_seq": z_seq,
        }

"""
Top-level ForecasterModel composition (human forecaster spec §20.1).

Pipeline (spec §30, short form):

**VSN → Latent CNN → Multi-Resolution xLSTM → Regime-Conditioned Fusion → Quantile Decoder**
"""

from __future__ import annotations

import numpy as np

from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_weights import ForecasterWeightBundle
from forecaster_model.models.decoder import forward_quantile_decoder, forward_quantile_decoder_weights
from forecaster_model.models.fusion import forward_regime_conditioned_fusion, forward_regime_conditioned_fusion_weights
from forecaster_model.models.latent_encoder import forward_latent_encoder, forward_latent_encoder_weights
from forecaster_model.models.multi_resolution_xlstm import (
    forward_multi_resolution_xlstm,
    forward_multi_resolution_xlstm_weights,
)
from forecaster_model.models.vsn import forward_vsn, forward_vsn_weights


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
        weight_bundle: ForecasterWeightBundle | None = None,
    ) -> dict[str, object]:
        _ = x_static
        _ = regime_seq
        if weight_bundle is not None:
            return self._forward_weights(x_obs, x_known, r_cur, weight_bundle)
        x_vsn, gates = forward_vsn(x_obs, self._rng)
        z_seq = forward_latent_encoder(x_vsn, self._rng, latent_dim=self.cfg.latent_width)
        L = z_seq.shape[0]
        scales = tuple(s for s in self.cfg.branch_scales if s <= L) or (1,)
        branches = forward_multi_resolution_xlstm(
            z_seq, scales, hidden_dim=self.cfg.recurrent_hidden_width, rng=self._rng
        )
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

    def _forward_weights(
        self,
        x_obs: np.ndarray,
        x_known: np.ndarray,
        r_cur: np.ndarray,
        wb: ForecasterWeightBundle,
    ) -> dict[str, object]:
        x_vsn, gates = forward_vsn_weights(x_obs, wb.vsn_W)
        z_seq = forward_latent_encoder_weights(
            x_vsn,
            wb.latent_weights[0],
            wb.latent_weights[1],
            wb.latent_weights[2],
            latent_dim=self.cfg.latent_width,
        )
        L = z_seq.shape[0]
        scales = tuple(s for s in self.cfg.branch_scales if s <= L) or (1,)
        branches = forward_multi_resolution_xlstm_weights(
            z_seq,
            scales,
            hidden_dim=self.cfg.recurrent_hidden_width,
            xlstm_by_scale=wb.xlstm_by_scale,
        )
        fused, alpha = forward_regime_conditioned_fusion_weights(branches, r_cur, wb.fusion_W)
        h_last = fused[-1]
        y_hat_q = forward_quantile_decoder_weights(h_last, x_known, self.cfg.quantiles, wb.decoder_W)
        return {
            "y_hat_q": y_hat_q,
            "gates": gates,
            "branch_outputs": branches,
            "fusion_weights": alpha,
            "z_seq": z_seq,
        }

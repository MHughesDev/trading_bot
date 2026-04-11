"""
Serializable NumPy weight bundles for `ForecasterModel` (FB-SPEC-02).

Training/orchestration can call `capture_forecaster_weights_from_seed` once and
`save_forecaster_weights` / `load_forecaster_weights`; serving loads NPZ and passes
the bundle into `forward_numpy_reference(..., weight_bundle=...)`.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from forecaster_model.config import ForecasterConfig


@dataclass(frozen=True)
class ForecasterWeightBundle:
    """All arrays needed for deterministic forward (no RNG). Shapes depend on `ForecasterConfig`."""

    vsn_W: np.ndarray
    latent_weights: tuple[np.ndarray, np.ndarray, np.ndarray]
    xlstm_by_scale: dict[int, dict[str, np.ndarray]]
    fusion_W: np.ndarray
    decoder_W: np.ndarray
    config_digest: dict[str, Any]

    def to_npz_dict(self) -> dict[str, np.ndarray]:
        d: dict[str, np.ndarray] = {
            "vsn_W": self.vsn_W,
            "latent_w0": self.latent_weights[0],
            "latent_w1": self.latent_weights[1],
            "latent_w2": self.latent_weights[2],
            "fusion_W": self.fusion_W,
            "decoder_W": self.decoder_W,
        }
        for s, wdict in sorted(self.xlstm_by_scale.items()):
            for k, arr in wdict.items():
                d[f"xlstm_s{s}_{k}"] = arr
        return d

    @classmethod
    def from_npz(cls, z: np.lib.npyio.NpzFile, *, cfg: ForecasterConfig) -> ForecasterWeightBundle:
        L = cfg.history_length
        scales = tuple(s for s in cfg.branch_scales if s <= L) or (1,)
        xlstm_by_scale: dict[int, dict[str, np.ndarray]] = {}
        for s in scales:
            xlstm_by_scale[s] = {
                "W_i": np.asarray(z[f"xlstm_s{s}_W_i"]),
                "W_f": np.asarray(z[f"xlstm_s{s}_W_f"]),
                "W_o": np.asarray(z[f"xlstm_s{s}_W_o"]),
                "W_c": np.asarray(z[f"xlstm_s{s}_W_c"]),
            }
        return cls(
            vsn_W=np.asarray(z["vsn_W"]),
            latent_weights=(
                np.asarray(z["latent_w0"]),
                np.asarray(z["latent_w1"]),
                np.asarray(z["latent_w2"]),
            ),
            xlstm_by_scale=xlstm_by_scale,
            fusion_W=np.asarray(z["fusion_W"]),
            decoder_W=np.asarray(z["decoder_W"]),
            config_digest={},
        )


def capture_forecaster_weights_from_seed(
    cfg: ForecasterConfig | None = None,
    *,
    seed: int = 42,
    f_obs: int = 11,
) -> ForecasterWeightBundle:
    """
    Draw one deterministic weight set matching `ForecasterModel(cfg, seed=seed).forward` RNG order.

    `f_obs` must match `build_observed_feature_matrix` column count (default windows → 11).
    """
    cfg = cfg or ForecasterConfig()
    rng = np.random.default_rng(seed)
    F = f_obs
    vsn_W = rng.normal(0, 0.05, size=(F, F))

    channels = (32, 64, cfg.latent_width)
    ks = (3, 5, 7)
    Fin = F
    latent_weights: list[np.ndarray] = []
    for ch_out, k in zip(channels, ks, strict=True):
        win_size = Fin * k
        latent_weights.append(rng.normal(0, 0.02, size=(win_size, ch_out)))
        Fin = ch_out

    L = cfg.history_length
    scales = tuple(s for s in cfg.branch_scales if s <= L) or (1,)
    hd = cfg.recurrent_hidden_width
    Fin_lstm = cfg.latent_width
    xlstm_by_scale: dict[int, dict[str, np.ndarray]] = {}
    for _s in scales:
        xlstm_by_scale[_s] = {
            "W_i": rng.normal(0, 0.05, size=(Fin_lstm + hd, hd)),
            "W_f": rng.normal(0, 0.05, size=(Fin_lstm + hd, hd)),
            "W_o": rng.normal(0, 0.05, size=(Fin_lstm + hd, hd)),
            "W_c": rng.normal(0, 0.05, size=(Fin_lstm + hd, hd)),
        }

    R = cfg.num_regime_dims
    fusion_W = rng.normal(0, 0.1, size=(R, len(scales)))

    H = cfg.forecast_horizon
    Fk = 6
    Qn = len(cfg.quantiles)
    D = hd
    decoder_W = rng.normal(0, 0.05, size=(D + Fk, Qn))

    return ForecasterWeightBundle(
        vsn_W=vsn_W,
        latent_weights=(latent_weights[0], latent_weights[1], latent_weights[2]),
        xlstm_by_scale=xlstm_by_scale,
        fusion_W=fusion_W,
        decoder_W=decoder_W,
        config_digest={
            "history_length": cfg.history_length,
            "forecast_horizon": H,
            "latent_width": cfg.latent_width,
            "recurrent_hidden_width": hd,
            "branch_scales": list(scales),
            "num_regime_dims": R,
            "quantiles": list(cfg.quantiles),
            "f_obs": f_obs,
            "seed": seed,
        },
    )


def save_forecaster_weights(path: str | Path, bundle: ForecasterWeightBundle) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(p, **bundle.to_npz_dict())


def load_forecaster_weights(path: str | Path, *, cfg: ForecasterConfig | None = None) -> ForecasterWeightBundle:
    p = Path(path)
    with np.load(p, allow_pickle=False) as z:
        c = cfg or ForecasterConfig()
        return ForecasterWeightBundle.from_npz(z, cfg=c)

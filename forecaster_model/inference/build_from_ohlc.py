"""Build `ForecastPacket` using spec methodology: normalize → regime → numpy reference forward."""

from __future__ import annotations

import logging
from datetime import UTC, datetime

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.features.normalization import rolling_zscore_causal
from forecaster_model.features.ohlc import build_observed_feature_matrix
from forecaster_model.features.time_future import known_future_features
from forecaster_model.calibration.conformal import MultiHorizonConformal, load_conformal_state
from forecaster_model.models.forecaster_weights import ForecasterWeightBundle
from forecaster_model.models.numpy_reference import forward_numpy_reference
from forecaster_model.regime.soft import soft_regime_from_returns

logger = logging.getLogger(__name__)


def build_forecast_packet_methodology(
    open_: np.ndarray,
    high: np.ndarray,
    low: np.ndarray,
    close: np.ndarray,
    volume: np.ndarray,
    *,
    cfg: ForecasterConfig | None = None,
    now: datetime | None = None,
    seed: int = 42,
    conformal_bundle: MultiHorizonConformal | None = None,
    conformal_state_path: str | None = None,
    weight_bundle: ForecasterWeightBundle | None = None,
) -> ForecastPacket:
    """
    End-to-end reference path aligned with human forecaster spec (NumPy reference model).

    - Causal rolling z-score on x_obs
    - Soft regime from log returns
    - Known-future cyclical features
    - VSN → CNN → multi-res RNN → fusion → quantile decoder
    - Optional **conformal** widening of quantile bands when `calibration_enabled` (FB-FR-PG3):
      load state from `conformal_state_path` if set, else in-memory bundle or fresh calibrators.
    """
    cfg = cfg or ForecasterConfig()
    c = np.asarray(close, dtype=np.float64).ravel()
    if len(c) < max(16, cfg.history_length):
        return _empty_packet(cfg, now)

    L = min(cfg.history_length, len(c))
    sl = slice(-L, None)
    o, h, lo, cl, vo = (
        open_[sl],
        high[sl],
        low[sl],
        close[sl],
        volume[sl],
    )
    x_obs = build_observed_feature_matrix(o, h, lo, cl, vo, windows=cfg.feature_windows)
    x_obs = rolling_zscore_causal(x_obs, window=min(256, L))
    lr = np.diff(np.log(np.maximum(c, 1e-12)))
    r_cur = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims)
    anchor = now or datetime.now(UTC)
    x_known = known_future_features(anchor, cfg.forecast_horizon, base_interval_seconds=cfg.base_interval_seconds)
    y_hat, diag = forward_numpy_reference(
        x_obs, x_known, r_cur, cfg, seed=seed, weight_bundle=weight_bundle
    )
    H = cfg.forecast_horizon
    q_lo = [float(y_hat[h, 0]) for h in range(H)]
    q_md = [float(y_hat[h, 1]) for h in range(H)]
    q_hi = [float(y_hat[h, 2]) for h in range(H)]
    iv = [q_hi[i] - q_lo[i] for i in range(H)]
    vol = float(np.std(lr[-32:])) if len(lr) >= 8 else 1.0
    conf = float(np.mean(np.abs(q_md)) / (np.mean(iv) + 1e-9))
    ens = [1e-8 * (i + 1) for i in range(H)]

    pkt = ForecastPacket(
        timestamp=anchor,
        horizons=list(range(1, H + 1)),
        q_low=q_lo,
        q_med=q_md,
        q_high=q_hi,
        interval_width=iv,
        regime_vector=r_cur.tolist(),
        confidence_score=conf,
        ensemble_variance=ens,
        ood_score=min(1.0, vol),
        forecast_diagnostics={
            "methodology": "numpy_reference",
            "weight_source": diag.get("weights", "rng"),
            **{k: v for k, v in diag.items() if k != "weights"},
        },
        packet_schema_version=1,
        source_checkpoint_id=None,
    )
    if cfg.calibration_enabled:
        bundle = conformal_bundle
        if bundle is None and conformal_state_path:
            try:
                bundle = load_conformal_state(conformal_state_path)
            except OSError as exc:
                logger.warning(
                    "conformal state not loaded from %s (%s); using ephemeral calibrators — "
                    "quantile bands may differ from offline training",
                    conformal_state_path,
                    exc,
                )
                bundle = None
        if bundle is None:
            bundle = MultiHorizonConformal.create(
                H,
                alpha=cfg.conformal_alpha,
                window_size=cfg.conformal_window_size,
            )
        pkt = bundle.apply_to_packet(pkt)
        pkt.forecast_diagnostics["conformal_state_source"] = conformal_state_path or "ephemeral"
    return pkt


def _empty_packet(cfg: ForecasterConfig, now: datetime | None) -> ForecastPacket:
    H = cfg.forecast_horizon
    z = [0.0] * H
    return ForecastPacket(
        timestamp=now or datetime.now(UTC),
        horizons=list(range(1, H + 1)),
        q_low=z.copy(),
        q_med=z.copy(),
        q_high=z.copy(),
        interval_width=z.copy(),
        regime_vector=[0.25] * cfg.num_regime_dims,
        confidence_score=0.0,
        ensemble_variance=z.copy(),
        ood_score=1.0,
        forecast_diagnostics={"methodology": "empty", "reason": "insufficient_history"},
        packet_schema_version=1,
        source_checkpoint_id=None,
    )

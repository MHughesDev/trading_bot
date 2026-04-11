"""Forecast ensemble: multiple members + variance in diagnostics (FB-FR-PG5)."""

from __future__ import annotations

import numpy as np

from app.contracts.forecast_packet import ForecastPacket
from forecaster_model.config import ForecasterConfig
from forecaster_model.models.forecaster_model import ForecasterModel


def build_ensemble_forecast_packet(packets: list[ForecastPacket]) -> ForecastPacket:
    """Merge member packets: quantiles averaged; ensemble_variance = per-horizon variance of q_med."""
    if not packets:
        raise ValueError("packets non-empty")
    base = packets[0]
    H = len(base.horizons)
    meds = np.array([[p.q_med[i] for i in range(H)] for p in packets], dtype=np.float64)
    q_med = [float(meds[:, h].mean()) for h in range(H)]
    ens_var = [float(meds[:, h].var()) for h in range(H)]
    lo = [float(np.mean([p.q_low[h] for p in packets])) for h in range(H)]
    hi = [float(np.mean([p.q_high[h] for p in packets])) for h in range(H)]
    iv = [hi[i] - lo[i] for i in range(H)]
    diag = dict(base.forecast_diagnostics)
    diag["ensemble_members"] = len(packets)
    diag["ensemble_aggregation"] = "mean_quantiles"
    return ForecastPacket(
        timestamp=base.timestamp,
        horizons=list(base.horizons),
        q_low=lo,
        q_med=q_med,
        q_high=hi,
        interval_width=iv,
        regime_vector=list(base.regime_vector),
        confidence_score=base.confidence_score,
        ensemble_variance=ens_var,
        ood_score=base.ood_score,
        forecast_diagnostics=diag,
        packet_schema_version=base.packet_schema_version,
        source_checkpoint_id=base.source_checkpoint_id,
    )


def forward_ensemble_numpy(
    x_obs: np.ndarray,
    x_known: np.ndarray,
    r_cur: np.ndarray,
    *,
    num_members: int,
    cfg: ForecasterConfig | None = None,
    base_seed: int = 42,
) -> tuple[np.ndarray, dict]:
    """Run N ForecasterModel instances with different seeds; return mean y_hat_q [H,Qn]."""
    cfg = cfg or ForecasterConfig()
    outs: list[np.ndarray] = []
    for m in range(num_members):
        model = ForecasterModel(cfg=cfg, seed=base_seed + m * 17)
        out = model.forward(x_obs, x_known, r_cur)
        outs.append(out["y_hat_q"])  # type: ignore[index]
    stack = np.stack(outs, axis=0)
    mean_q = stack.mean(axis=0)
    var_h = stack.var(axis=0).mean(axis=1)
    return mean_q, {"member_var_per_horizon": var_h.tolist(), "num_members": num_members}

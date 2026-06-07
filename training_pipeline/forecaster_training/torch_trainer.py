"""
PyTorch forecaster training stack (FB-FR-PG2; full hot-path torch: **FB-FR-P0**, delivered NumPy path: **FB-FR-CORE**).

Optional dependency: `torch`. When unavailable, `train_forecaster_stub` documents the contract
and writes a JSON checkpoint only (no neural weights).

`train_forecaster_torch` trains the real serving graph (`ForecasterTorchMLP`) with a
multi-quantile pinball loss on causal OHLC features that match the inference path exactly
(`build_observed_feature_matrix` + causal z-score, soft regime, known-future), so the saved
`forecaster_torch.pt` is loadable by `load_torch_forecaster_checkpoint` and used directly by
`DecisionPipeline`. Targets are realized cumulative log-returns at each horizon.
"""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import numpy as np

from legacy.decision_pipeline.forecaster_model.config import ForecasterConfig
from legacy.decision_pipeline.forecaster_model.features.normalization import rolling_zscore_causal
from legacy.decision_pipeline.forecaster_model.features.ohlc import build_observed_feature_matrix
from legacy.decision_pipeline.forecaster_model.features.time_future import known_future_features
from legacy.decision_pipeline.forecaster_model.regime.soft import soft_regime_from_returns
from training_pipeline.forecaster_training.checkpoint import save_json_checkpoint
from training_pipeline.forecaster_training.device import resolve_torch_device


def train_forecaster_stub(
    *,
    artifact_dir: str | Path,
    epochs: int = 1,
    patience: int = 5,
) -> dict[str, Any]:
    """
    Placeholder training loop: records hyperparameters and early-stopping config for CI/docs.

    When `torch` is installed, `train_forecaster_torch` runs a real loop on the serving graph.
    """
    artifact_dir = Path(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    meta = {
        "trainer": "stub",
        "epochs": epochs,
        "early_stopping_patience": patience,
        "note": "Install torch and call train_forecaster_torch for the architecture-spec hot path",
    }
    save_json_checkpoint(artifact_dir / "forecaster_train_meta.json", meta)
    return meta


def _extract_ohlcv(
    bars: Any,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray, list[datetime] | None]:
    """Pull OHLCV (+ optional timestamps) from a Polars DataFrame, mapping, or arrays tuple."""
    cols = ("open", "high", "low", "close", "volume")
    # Polars DataFrame (duck-typed to avoid importing polars here)
    if hasattr(bars, "columns") and hasattr(bars, "get_column"):
        o, h, lo, c, v = (bars.get_column(k).to_numpy().astype(np.float64) for k in cols)
        ts: list[datetime] | None = None
        for tcol in ("timestamp", "ts", "time"):
            if tcol in bars.columns:
                raw = bars.get_column(tcol).to_list()
                ts = [t if isinstance(t, datetime) else None for t in raw]
                if any(t is None for t in ts):
                    ts = None
                break
        return o, h, lo, c, v, ts
    if isinstance(bars, dict):
        o, h, lo, c, v = (np.asarray(bars[k], dtype=np.float64).ravel() for k in cols)
        return o, h, lo, c, v, None
    o, h, lo, c, v = (np.asarray(a, dtype=np.float64).ravel() for a in bars)
    return o, h, lo, c, v, None


def _synth_ohlc(n: int, *, seed: int) -> tuple[np.ndarray, ...]:
    """Deterministic structured random-walk OHLCV when no real bars are supplied."""
    rng = np.random.default_rng(seed)
    steps = rng.standard_normal(n) * 0.004 + 0.00005  # minute-scale drift+vol
    close = 100.0 * np.exp(np.cumsum(steps))
    open_ = np.concatenate([[close[0]], close[:-1]])
    noise = np.abs(rng.standard_normal(n)) * 0.002 * close
    high = np.maximum(open_, close) + noise
    low = np.minimum(open_, close) - noise
    volume = 1e6 * (1.0 + 0.1 * rng.standard_normal(n))
    return open_, high, low, close, np.abs(volume)


def build_torch_training_samples(
    bars: Any,
    cfg: ForecasterConfig,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Build (X_obs, X_known, R, Y) stacks matching the inference feature recipe.

    For each anchor ``t`` with ``L`` bars of history and ``H`` bars of future:
    - ``X_obs[t]``  = causal z-scored observed feature matrix of the last L bars  → [L, F_obs]
    - ``X_known[t]``= known-future cyclical features at the anchor time           → [H, 6]
    - ``R[t]``      = soft regime vector from log-returns up to t                  → [R]
    - ``Y[t]``      = realized cumulative log-returns at horizons 1..H             → [H]
    """
    o, h, lo, c, v, ts = _extract_ohlcv(bars)
    n = len(c)
    L = int(cfg.history_length)
    H = int(cfg.forecast_horizon)
    logc = np.log(np.maximum(c, 1e-12))
    zwin = min(256, L)

    x_obs_list: list[np.ndarray] = []
    x_known_list: list[np.ndarray] = []
    r_list: list[np.ndarray] = []
    y_list: list[np.ndarray] = []
    base_anchor = datetime(2024, 1, 1, tzinfo=UTC)
    for t in range(L - 1, n - H):
        sl = slice(t - L + 1, t + 1)
        x_obs = build_observed_feature_matrix(
            o[sl], h[sl], lo[sl], c[sl], v[sl], windows=cfg.feature_windows
        )
        x_obs = rolling_zscore_causal(x_obs, window=zwin)
        lr = np.diff(logc[: t + 1])
        r_cur = soft_regime_from_returns(lr, num_regimes=cfg.num_regime_dims)
        anchor = (
            ts[t] if ts is not None else base_anchor + timedelta(seconds=cfg.base_interval_seconds * t)
        )
        x_known = known_future_features(
            anchor, H, base_interval_seconds=cfg.base_interval_seconds
        )
        y = np.array([logc[t + k] - logc[t] for k in range(1, H + 1)], dtype=np.float64)
        if not (np.all(np.isfinite(x_obs)) and np.all(np.isfinite(y))):
            continue
        x_obs_list.append(x_obs)
        x_known_list.append(x_known)
        r_list.append(r_cur)
        y_list.append(y)

    if not x_obs_list:
        raise ValueError("not enough bars to build any training sample for the given config")
    return (
        np.stack(x_obs_list).astype(np.float32),
        np.stack(x_known_list).astype(np.float32),
        np.stack(r_list).astype(np.float32),
        np.stack(y_list).astype(np.float32),
    )


def train_forecaster_torch(
    *,
    artifact_dir: str | Path,
    bars: Any | None = None,
    cfg: ForecasterConfig | None = None,
    epochs: int = 8,
    learning_rate: float = 1e-3,
    batch_size: int = 64,
    device: str | None = "auto",
    seed: int = 0,
    min_samples: int = 16,
) -> dict[str, Any]:
    """
    Train the real ``ForecasterTorchMLP`` with multi-quantile pinball loss; else raises ImportError.

    Trains on ``bars`` (Polars DataFrame / OHLCV mapping / arrays) when provided, otherwise on a
    deterministic structured random walk so the call always yields a serving-loadable checkpoint.
    Saves ``forecaster_torch.pt`` (state_dict + cfg + device) — a drop-in for the inference path.
    ``device`` follows ``resolve_torch_device`` (default ``auto``: CUDA when available, else CPU).
    """
    try:
        import torch
    except ImportError as e:
        raise ImportError("Install trading-bot[models_torch] for PyTorch forecaster training") from e

    from legacy.decision_pipeline.forecaster_model.models.torch_forecaster_net import build_torch_forecaster

    cfg = cfg or ForecasterConfig()
    artifact_dir = Path(artifact_dir)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    dev = torch.device(resolve_torch_device(device))
    torch.manual_seed(seed)

    if bars is None:
        need = cfg.history_length + cfg.forecast_horizon + max(min_samples, 64) + 8
        bars = _synth_ohlc(need, seed=seed)

    x_obs, x_known, r_cur, y = build_torch_training_samples(bars, cfg)
    if x_obs.shape[0] < min_samples:
        raise ValueError(
            f"only {x_obs.shape[0]} samples (< min_samples={min_samples}); supply more bars"
        )

    xo = torch.from_numpy(x_obs).to(dev)
    xk = torch.from_numpy(x_known).to(dev)
    rr = torch.from_numpy(r_cur).to(dev)
    yt = torch.from_numpy(y).to(dev)
    taus = torch.tensor(cfg.quantiles, dtype=torch.float32, device=dev)  # [Qn]

    model = build_torch_forecaster(cfg).to(dev)
    opt = torch.optim.Adam(model.parameters(), lr=learning_rate)

    n = xo.shape[0]
    last_loss = float("nan")
    first_loss = float("nan")
    model.train()
    for ep in range(max(1, epochs)):
        perm = torch.randperm(n, device=dev)
        ep_losses: list[float] = []
        for i in range(0, n, batch_size):
            idx = perm[i : i + batch_size]
            opt.zero_grad()
            pred = model(xo[idx], xk[idx], rr[idx])  # [B, H, Qn]
            target = yt[idx].unsqueeze(-1)  # [B, H, 1]
            diff = target - pred  # broadcast over Qn
            pinball = torch.maximum(taus * diff, (taus - 1.0) * diff)  # [B, H, Qn]
            loss = pinball.mean()
            loss.backward()
            opt.step()
            ep_losses.append(float(loss.detach().cpu()))
        last_loss = float(np.mean(ep_losses))
        if ep == 0:
            first_loss = last_loss

    path = artifact_dir / "forecaster_torch.pt"
    torch.save(
        {
            "state_dict": model.state_dict(),
            "forecaster_config": cfg,
            "device": str(dev),
            "trainer": "torch_pinball_mlp",
        },
        path,
    )
    meta = {
        "trainer": "torch_pinball_mlp",
        "weights": str(path),
        "samples": int(n),
        "epochs": int(max(1, epochs)),
        "first_epoch_loss": first_loss,
        "final_epoch_loss": last_loss,
        "quantiles": list(cfg.quantiles),
        "device": str(dev),
        "data": "real_bars" if not isinstance(bars, tuple) else "synthetic_random_walk",
    }
    save_json_checkpoint(artifact_dir / "forecaster_train_meta.json", meta)
    return meta

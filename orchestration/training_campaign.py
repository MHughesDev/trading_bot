"""
Real-data-only training driver (initial offline + nightly maintenance specs).

No synthetic arrays. Fetches Kraken OHLC/trades (see ``real_data_bars``), walk-forward splits, fits quantile forecaster,
runs heuristic policy rollout with real returns (RL placeholder until PPO/SAC).

**Offline vs live forecaster:** this path fits **sklearn `QuantileRegressor`** and saves
`forecaster_quantile_real.joblib`. Runtime **`DecisionPipeline`** builds **`ForecastPacket`**
via **NumPy `ForecasterModel`** (`build_forecast_packet_methodology`) — not the same weights.
Pinball metrics here **do not** equal live packet distribution until **FB-SPEC-02** wires
checkpoints or a shared artifact.
"""

from __future__ import annotations

import logging
import os
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any, Literal

import numpy as np
import polars as pl

from app.config.settings import AppSettings, load_settings
from app.runtime.system_power import is_on, sync_from_disk
from forecaster_model.config import ForecasterConfig
from forecaster_model.training.metrics import mean_pinball_loss
from forecaster_model.training.real_data_fit import (
    QuantileForecasterArtifact,
    fit_quantile_forecaster_from_bars,
    predict_quantile_forecast_packet,
    save_training_report,
)
from orchestration.real_data_bars import dataset_snapshot_id, fetch_symbol_bars_sync, write_snapshot_manifest
from orchestration.rl_real_data_eval import run_heuristic_rollout_on_range
from orchestration.training_spec_constants import (
    CAMPAIGN_FORECASTER_RUNS,
    CAMPAIGN_FORECASTER_SEEDS,
    CAMPAIGN_HISTORY_LENGTH,
    CAMPAIGN_FORECAST_HORIZON,
    CAMPAIGN_MAX_EPOCHS,
    CAMPAIGN_RL_ENV_STEPS_MIN,
    CAMPAIGN_RL_RUNS,
    CAMPAIGN_RL_SEEDS,
    CAMPAIGN_WALK_FORWARD_SPLITS,
    NIGHTLY_FORECASTER_RUNS,
    NIGHTLY_FORECASTER_SEEDS,
    NIGHTLY_MAX_EPOCHS,
    NIGHTLY_RL_ENV_STEPS_PREFERRED,
    NIGHTLY_RL_RUNS,
    NIGHTLY_RL_SEEDS,
)
from orchestration.promotion import decide_forecaster_promotion_stub, write_promotion_sidecar
from orchestration.walkforward_triple import triple_splits

logger = logging.getLogger(__name__)


def _cfg_from_spec(
    mode: Literal["initial", "nightly"],
    *,
    settings: AppSettings,
) -> ForecasterConfig:
    """
    Geometry matches runtime `ForecasterConfig` defaults (`forecaster_model/config`) so
    offline pinball / quantile fit windows align with `build_forecast_packet_methodology`
    (FB-AUDIT-01). Initial and nightly both use full campaign horizon/history from spec constants.
    """
    base_sec = max(1, int(settings.training_data_granularity_seconds))
    return ForecasterConfig(
        history_length=CAMPAIGN_HISTORY_LENGTH,
        forecast_horizon=CAMPAIGN_FORECAST_HORIZON,
        base_interval_seconds=base_sec,
        feature_windows=(4, 16, 64),
        num_regime_dims=4,
        quantiles=(0.1, 0.5, 0.9),
    )


def _eval_pinball_on_range(
    bars: pl.DataFrame,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    eval_range: range,
) -> dict[str, float]:
    o = bars["open"].to_numpy()
    h = bars["high"].to_numpy()
    lo = bars["low"].to_numpy()
    cl = bars["close"].to_numpy()
    vo = bars["volume"].to_numpy()
    horizons = artifact.horizons
    preds: list[np.ndarray] = []
    targets: list[np.ndarray] = []
    L = cfg.history_length
    max_h = max(horizons)
    for t in range(eval_range.start + L - 1, min(eval_range.stop - 1 - max_h, len(cl) - 1 - max_h)):
        if t not in eval_range:
            continue
        sl = slice(t - L + 1, t + 1)
        if "timestamp" in bars.columns:
            ts_raw = bars.get_column("timestamp").to_numpy()[t]
            ts = ts_raw if isinstance(ts_raw, datetime) else datetime.fromtimestamp(float(ts_raw), tz=UTC)
        else:
            ts = datetime.now(UTC)
        pkt = predict_quantile_forecast_packet(o[sl], h[sl], lo[sl], cl[sl], vo[sl], artifact, cfg, now_ts=ts)
        y = np.array(
            [float(np.log(cl[t + hp] / max(cl[t], 1e-12))) for hp in horizons],
            dtype=np.float64,
        )
        preds.append(np.column_stack([pkt.q_low, pkt.q_med, pkt.q_high]))
        targets.append(y)
    if not preds:
        return {"pinball_mean": float("nan"), "n": 0.0}
    P = np.stack(preds, axis=0)  # [N, H, 3]
    Y = np.stack(targets, axis=0)  # [N, H]
    losses: list[float] = []
    for j in range(len(horizons)):
        losses.append(mean_pinball_loss(Y[:, j], P[:, j, :], artifact.quantiles))
    return {"pinball_mean": float(np.mean(losses)), "n": float(P.shape[0])}


def run_training_campaign(
    *,
    mode: Literal["initial", "nightly"] = "nightly",
    symbol: str | None = None,
    artifact_dir: Path | None = None,
    settings: AppSettings | None = None,
    lookback_days: int = 120,
    granularity_seconds: int | None = None,
) -> dict[str, Any]:
    """
    Execute real-data training per spec. Writes artifacts under artifact_dir.
    """
    sync_from_disk()
    if not is_on():
        logger.warning("system power OFF — skipping training campaign")
        return {"skipped": True, "reason": "system_power_off"}
    s = settings or load_settings()
    sym = symbol or (s.market_data_symbols[0] if s.market_data_symbols else "BTC-USD")
    gsec = (
        int(granularity_seconds)
        if granularity_seconds is not None
        else max(1, int(s.training_data_granularity_seconds))
    )
    out = Path(artifact_dir or Path("models/artifacts_training"))
    out.mkdir(parents=True, exist_ok=True)
    cfg = _cfg_from_spec(mode, settings=s)

    end = datetime.now(UTC)
    start = end - timedelta(days=lookback_days)
    snap_id = dataset_snapshot_id(sym, start, end, gsec)
    manifest_path = out / f"data_snapshot_{snap_id}.json"
    write_snapshot_manifest(
        manifest_path,
        {
            "symbol": sym,
            "start": start.isoformat(),
            "end": end.isoformat(),
            "granularity_seconds": gsec,
            "snapshot_id": snap_id,
        },
    )
    logger.info("fetching real candles %s %s → %s", sym, start, end)
    bars = fetch_symbol_bars_sync(sym, start, end, granularity_seconds=gsec)
    bars.write_parquet(out / "bars.parquet")

    n = bars.height
    splits = triple_splits(n, CAMPAIGN_WALK_FORWARD_SPLITS if mode == "initial" else 1)
    if mode == "nightly" and len(splits) < 1:
        raise RuntimeError("nightly mode requires at least one triple split")

    forecaster_seeds = CAMPAIGN_FORECASTER_SEEDS if mode == "initial" else NIGHTLY_FORECASTER_SEEDS
    n_fore_runs = CAMPAIGN_FORECASTER_RUNS if mode == "initial" else NIGHTLY_FORECASTER_RUNS
    max_epochs = CAMPAIGN_MAX_EPOCHS if mode == "initial" else NIGHTLY_MAX_EPOCHS
    rl_seeds = CAMPAIGN_RL_SEEDS if mode == "initial" else NIGHTLY_RL_SEEDS
    n_rl_runs = CAMPAIGN_RL_RUNS if mode == "initial" else NIGHTLY_RL_RUNS
    rl_steps = CAMPAIGN_RL_ENV_STEPS_MIN if mode == "initial" else NIGHTLY_RL_ENV_STEPS_PREFERRED

    _ = max_epochs  # sklearn quantile fit is single-pass; epochs reserved for future torch

    fore_results: list[dict[str, Any]] = []
    best_artifact: QuantileForecasterArtifact | None = None
    best_score = float("inf")

    run_idx = 0
    for sp_idx, trip in enumerate(splits):
        for seed in forecaster_seeds:
            if run_idx >= n_fore_runs:
                break
            art = fit_quantile_forecaster_from_bars(
                bars,
                cfg,
                train_range=trip.train,
                data_snapshot_id=snap_id,
                seed=int(seed),
            )
            val_metrics = _eval_pinball_on_range(bars, art, cfg, trip.validation)
            hold_metrics = _eval_pinball_on_range(bars, art, cfg, trip.test)
            score = val_metrics["pinball_mean"] + hold_metrics["pinball_mean"]
            rec = {
                "split": sp_idx,
                "seed": seed,
                "val": val_metrics,
                "holdout": hold_metrics,
                "aggregate_score": score,
            }
            fore_results.append(rec)
            if score < best_score and not np.isnan(score):
                best_score = score
                best_artifact = art
            run_idx += 1
        if run_idx >= n_fore_runs:
            break

    if best_artifact is None:
        raise RuntimeError("forecaster training failed: no valid candidate")

    fc_path = out / "forecaster_quantile_real.joblib"
    best_artifact.save(fc_path)

    rl_results: list[dict[str, Any]] = []
    rl_idx = 0
    # Initial campaign: 2 algorithm families × 3 splits × 3 seeds = 18 (heuristic placeholders for PPO/SAC)
    alg_tags = ("ppo_placeholder", "sac_placeholder") if mode == "initial" else ("nightly",)
    for alg in alg_tags:
        for sp_idx, trip in enumerate(splits):
            for seed in rl_seeds:
                if rl_idx >= n_rl_runs:
                    break
                m = run_heuristic_rollout_on_range(
                    bars,
                    trip.train,
                    best_artifact,
                    cfg,
                    max_steps=rl_steps,
                )
                rl_results.append(
                    {
                        "split": sp_idx,
                        "algorithm": alg,
                        "seed": seed,
                        "metrics": m.__dict__,
                    }
                )
                rl_idx += 1
            if rl_idx >= n_rl_runs:
                break
        if rl_idx >= n_rl_runs:
            break

    report = {
        "mode": mode,
        "symbol": sym,
        "data_snapshot_id": snap_id,
        "data_manifest": str(manifest_path),
        "bars_rows": n,
        "forecaster_artifact": str(fc_path),
        "forecaster_runs": fore_results,
        "best_forecaster_aggregate_score": best_score,
        "rl_runs": rl_results,
        "spec_notes": {
            "forecaster": "quantile_ohlc_v1 sklearn on real Kraken candles/trades only",
            "rl": "heuristic policy rollout on real returns; PPO/SAC backlog",
        },
    }
    save_training_report(out / "training_report.json", report)
    prev = os.environ.get("NM_PREVIOUS_FORECASTER_CHAMPION_PATH")
    prom = decide_forecaster_promotion_stub(report=report, previous_champion_path=prev)
    prom_path = write_promotion_sidecar(out, prom)
    report["promotion_decision"] = prom.to_dict()
    logger.info(
        "training complete: forecaster=%s report=%s promotion=%s",
        fc_path,
        out / "training_report.json",
        prom_path,
    )
    return report


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    run_training_campaign(mode="nightly")


if __name__ == "__main__":
    main()

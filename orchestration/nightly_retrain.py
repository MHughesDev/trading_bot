"""
Nightly / scheduled training entry — **real Coinbase candles only** (no synthetic arrays).

Implements cadence from `docs/Human Provided Specs/NIGHTLY_TRAINING_AND_REFRESH_SPEC.MD`:
forecaster refresh → evaluation → heuristic RL rollout on real prices (PPO/SAC backlog).

Promotion: never automatic; artifacts written under `NM_TRAINING_ARTIFACT_DIR` or `models/artifacts_training/`.
"""

from __future__ import annotations

import argparse
import logging
import os
from pathlib import Path

from app.config.settings import AppSettings, load_settings
from app.runtime.asset_model_registry import list_symbols as list_manifest_symbols

from orchestration.training_campaign import run_training_campaign

logger = logging.getLogger(__name__)


def run_nightly_training_job(
    *,
    settings: AppSettings | None = None,
    artifact_dir: Path | None = None,
    lookback_days: int | None = None,
) -> dict:
    """
    **Nightly** real-data training (FB-AP-035 / FB-AP-036).

    When ``scheduler_nightly_per_asset_forecaster`` is true (default), runs **one** nightly
    campaign per **initialized** asset (symbols with a manifest). Artifacts go under
    ``<base>/nightly/<canonical_symbol>/``. Otherwise runs a single campaign for the first
    configured market symbol (legacy behavior).
    """
    p = artifact_dir or Path(os.environ.get("NM_TRAINING_ARTIFACT_DIR", "models/artifacts_training"))
    s = settings or load_settings()
    lb = lookback_days if lookback_days is not None else int(os.environ.get("NM_TRAINING_LOOKBACK_DAYS", "90"))

    if s.scheduler_nightly_per_asset_forecaster:
        syms = sorted(list_manifest_symbols())
        if not syms:
            logger.info("nightly training: no initialized assets (empty manifest registry)")
            return {
                "mode": "nightly",
                "skipped": True,
                "reason": "no_manifest_symbols",
                "symbols": [],
                "reports": {},
            }
        base = p.resolve()
        reports: dict[str, dict] = {}
        for sym in syms:
            out_dir = base / "nightly" / sym
            logger.info("nightly training: symbol=%s artifact_dir=%s", sym, out_dir)
            reports[sym] = run_training_campaign(
                mode="nightly",
                symbol=sym,
                artifact_dir=out_dir,
                settings=s,
                lookback_days=lb,
            )
        return {
            "mode": "nightly",
            "per_asset": True,
            "symbols": syms,
            "reports": reports,
        }

    return run_training_campaign(
        mode="nightly",
        artifact_dir=p,
        settings=s,
        lookback_days=lb,
    )


def nightly_flow_entrypoint() -> None:
    """CLI: `python -m orchestration.nightly_retrain` (no args — nightly defaults)."""
    logging.basicConfig(level=logging.INFO)
    report = run_nightly_training_job()
    logger.info("nightly training report keys: %s", list(report.keys()))


def main() -> None:
    parser = argparse.ArgumentParser(description="Real-data training (nightly or initial campaign)")
    parser.add_argument(
        "--mode",
        choices=("nightly", "initial"),
        default="nightly",
        help="nightly maintenance vs initial offline campaign (larger budgets)",
    )
    parser.add_argument(
        "--artifact-dir",
        type=Path,
        default=None,
        help="Output directory (default: NM_TRAINING_ARTIFACT_DIR or models/artifacts_training)",
    )
    parser.add_argument("--symbol", type=str, default=None, help="Override product id (default: first NM market symbol)")
    parser.add_argument("--lookback-days", type=int, default=None, help="History window in days")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO)
    settings = load_settings()
    ad = args.artifact_dir or Path(os.environ.get("NM_TRAINING_ARTIFACT_DIR", "models/artifacts_training"))
    lb = args.lookback_days
    if lb is None:
        lb = 180 if args.mode == "initial" else int(os.environ.get("NM_TRAINING_LOOKBACK_DAYS", "90"))
    run_training_campaign(
        mode=args.mode,
        symbol=args.symbol,
        artifact_dir=ad,
        settings=settings,
        lookback_days=lb,
    )


if __name__ == "__main__":
    main()

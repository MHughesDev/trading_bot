"""End-to-end decision: regime (packet) → forecast packet → PolicySystem → proposal (before risk).

Canonical path matches `docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD` §5:
features → forecaster (VSN → CNN → multi-res xLSTM → fusion → quantiles) → ForecastPacket →
PolicySystem → RiskEngine contracts.
"""

from __future__ import annotations

import logging
from decimal import Decimal
from pathlib import Path

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from decision_engine.spec_policy_proposal import run_spec_policy_step
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.inference.stub import ohlc_arrays_from_feature_row

logger = logging.getLogger(__name__)

# Log once per process: hot path is NumPy reference + heuristic policy until FB-SPEC-02.
_serving_mode_logged = False


def _feature_vector(values: dict[str, float], dim: int = 32) -> np.ndarray:
    vec = np.zeros(dim, dtype=np.float64)
    keys = sorted(values.keys())
    for i, k in enumerate(keys[:dim]):
        vec[i] = float(values[k])
    return vec


def _regime_output_from_packet(pkt: ForecastPacket) -> RegimeOutput:
    """Map soft regime vector (length 4) to `RegimeOutput` for observability."""
    probs = list(pkt.regime_vector)
    if len(probs) < 4:
        probs = (probs + [0.25] * 4)[:4]
    s = sum(probs) or 1.0
    p = [x / s for x in probs[:4]]
    idx = int(np.argmax(p))
    sem_map = (SemanticRegime.BULL, SemanticRegime.BEAR, SemanticRegime.VOLATILE, SemanticRegime.SIDEWAYS)
    sem = sem_map[idx] if idx < 4 else SemanticRegime.SIDEWAYS
    return RegimeOutput(
        state_index=idx,
        semantic=sem,
        probabilities=p,
        confidence=float(max(p)),
    )


def _forecaster_config_from_env(conformal_path: str | None) -> ForecasterConfig:
    cfg = ForecasterConfig()
    if conformal_path and Path(conformal_path).is_file():
        cfg.calibration_enabled = True
    return cfg


class DecisionPipeline:
    def __init__(self, settings: AppSettings | None = None) -> None:
        self._settings = settings or load_settings()
        self._last_forecast_packet: ForecastPacket | None = None

    @staticmethod
    def _log_serving_mode_once(settings: AppSettings) -> None:
        global _serving_mode_logged
        if _serving_mode_logged:
            return
        _serving_mode_logged = True
        logger.info(
            "decision pipeline serving mode: NumPy ForecasterModel + heuristic PolicySystem "
            "(no PyTorch/checkpoint weights on hot path until FB-SPEC-02); "
            "NM_MODELS_FORECASTER_CHECKPOINT_ID=%s",
            settings.models_forecaster_checkpoint_id or "(unset)",
        )

    @property
    def last_forecast_packet(self) -> ForecastPacket | None:
        """Last `ForecastPacket` built on the hot path (master spec §5)."""
        return self._last_forecast_packet

    def step(
        self,
        symbol: str,
        feature_row: dict[str, float],
        spread_bps: float,
        risk: RiskState,
        *,
        mid_price: float | None = None,
        portfolio_equity_usd: float | None = None,
        position_signed_qty: Decimal | None = None,
    ) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None]:
        _ = _feature_vector(feature_row)  # reserved for future feature-cache alignment
        self._log_serving_mode_once(self._settings)

        conf_path = self._settings.models_forecaster_conformal_state_path
        cfg = _forecaster_config_from_env(conf_path)
        o, h, lo, cl, vo = ohlc_arrays_from_feature_row(feature_row, history_len=cfg.history_length)
        pkt = build_forecast_packet_methodology(
            o,
            h,
            lo,
            cl,
            vo,
            cfg=cfg,
            conformal_state_path=conf_path,
        )
        pkt.forecast_diagnostics["symbol"] = symbol
        pkt.forecast_diagnostics["pipeline"] = "master_spec"
        pkt.packet_schema_version = 1
        cid = self._settings.models_forecaster_checkpoint_id
        pkt.source_checkpoint_id = cid
        self._last_forecast_packet = pkt

        regime_out = _regime_output_from_packet(pkt)
        mp = float(mid_price) if mid_price is not None else float(feature_row.get("close", 1.0))
        eq = float(portfolio_equity_usd) if portfolio_equity_usd is not None else 100_000.0
        fc, route, action = run_spec_policy_step(
            symbol,
            pkt,
            settings=self._settings,
            app_risk=risk,
            mid_price=mp,
            spread_bps=spread_bps,
            portfolio_equity_usd=eq,
            position_signed_qty=position_signed_qty,
        )
        return regime_out, fc, route, action

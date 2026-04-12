"""End-to-end decision: regime (packet) → forecast packet → PolicySystem → proposal (before risk).

Canonical path matches `docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD` §5:
features → forecaster (VSN → CNN → multi-res xLSTM → fusion → quantiles) → ForecastPacket →
PolicySystem → RiskEngine contracts.
"""

from __future__ import annotations

import logging
from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from data_plane.storage.asset_model_registry import resolve_manifest_for_symbol
from decision_engine.spec_policy_proposal import run_spec_policy_step
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.inference.stub import ohlc_arrays_from_feature_row
from forecaster_model.models.forecaster_weights import load_forecaster_weights
from policy_model.policy.policy_network import PolicyNetwork
from policy_model.system import PolicySystem

logger = logging.getLogger(__name__)

# Log once per process: serving mode (RNG vs NPZ weights).
_serving_mode_logged = False


def _feature_vector(values: dict[str, float], dim: int = 32) -> np.ndarray:
    vec = np.zeros(dim, dtype=np.float64)
    keys = sorted(values.keys())
    for i, k in enumerate(keys[:dim]):
        vec[i] = float(values[k])
    return vec


def _abstain_forecast_packet(
    *,
    symbol: str,
    reason: str,
    manifest_id: str | None,
) -> ForecastPacket:
    """Neutral packet when manifest binding fails (FB-AP-003) — no trade via downstream policy."""
    cfg = ForecasterConfig()
    h = cfg.forecast_horizon
    z = [0.0] * h
    return ForecastPacket(
        timestamp=datetime.now(UTC),
        horizons=list(range(1, h + 1)),
        q_low=z.copy(),
        q_med=z.copy(),
        q_high=z.copy(),
        interval_width=z.copy(),
        regime_vector=[0.25] * cfg.num_regime_dims,
        confidence_score=0.0,
        ensemble_variance=z.copy(),
        ood_score=1.0,
        forecast_diagnostics={
            "methodology": "abstain",
            "reason": reason,
            "symbol": symbol,
            "manifest_binding": manifest_id,
            "pipeline": "master_spec",
        },
        packet_schema_version=1,
        source_checkpoint_id=None,
    )


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
        self._forecaster_weight_bundle = _load_forecaster_bundle_if_configured(self._settings)
        self._torch_model, self._torch_device, self._torch_cfg = _load_torch_forecaster_if_configured(
            self._settings
        )
        self._policy_system: PolicySystem | None = _load_policy_system_if_configured(self._settings)

    @staticmethod
    def _log_serving_mode_once(settings: AppSettings) -> None:
        global _serving_mode_logged
        if _serving_mode_logged:
            return
        _serving_mode_logged = True
        fw = settings.models_forecaster_weights_path
        ft = settings.models_forecaster_torch_path
        pp = settings.models_policy_mlp_path
        has_torch = bool(ft and Path(ft).is_file())
        has_npz = bool(fw and Path(fw).is_file())
        has_p = bool(pp and Path(pp).is_file())
        if has_torch:
            fmode = "pytorch_mlp"
        elif has_npz:
            fmode = "npz_weights"
        else:
            fmode = "numpy_rng"
        logger.info(
            "decision pipeline serving mode: forecaster=%s policy=%s "
            "(ForecastPacket + PolicySystem; optional PyTorch/NPZ); "
            "NM_MODELS_FORECASTER_CHECKPOINT_ID=%s",
            fmode,
            "mlp_npz" if has_p else "heuristic",
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

        manifest = resolve_manifest_for_symbol(self._settings, symbol)
        if manifest is not None:
            try:
                manifest.assert_matches_decision_symbol(symbol)
            except ValueError as exc:
                logger.error(
                    "forecaster manifest binding failed: %s (decision_symbol=%r manifest_id=%r)",
                    exc,
                    symbol,
                    manifest.manifest_id,
                )
                pkt = _abstain_forecast_packet(
                    symbol=symbol,
                    reason="manifest_symbol_mismatch",
                    manifest_id=manifest.manifest_id,
                )
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
                    policy_system=self._policy_system,
                )
                return regime_out, fc, route, action

        conf_path = self._settings.models_forecaster_conformal_state_path
        base_cfg = _forecaster_config_from_env(conf_path)
        if self._torch_model is not None and self._torch_cfg is not None:
            cfg = self._torch_cfg
            if conf_path and Path(conf_path).is_file():
                cfg.calibration_enabled = True
        else:
            cfg = base_cfg
        bar_sec = max(1, int(self._settings.market_data_bar_interval_seconds))
        cfg.base_interval_seconds = bar_sec
        o, h, lo, cl, vo = ohlc_arrays_from_feature_row(feature_row, history_len=cfg.history_length)
        pkt = build_forecast_packet_methodology(
            o,
            h,
            lo,
            cl,
            vo,
            cfg=cfg,
            conformal_state_path=conf_path,
            weight_bundle=self._forecaster_weight_bundle if self._torch_model is None else None,
            torch_model=self._torch_model,
            torch_device=self._torch_device,
        )
        pkt.forecast_diagnostics["symbol"] = symbol
        pkt.forecast_diagnostics["pipeline"] = "master_spec"
        if manifest is not None:
            pkt.forecast_diagnostics["asset_manifest_id"] = manifest.manifest_id
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
            policy_system=self._policy_system,
        )
        return regime_out, fc, route, action


def _load_torch_forecaster_if_configured(settings: AppSettings):
    p = settings.models_forecaster_torch_path
    if not p or not Path(p).is_file():
        return None, None, None
    try:
        from forecaster_model.inference.torch_infer import load_torch_forecaster_checkpoint

        model, dev, tcfg = load_torch_forecaster_checkpoint(p)
        return model, dev, tcfg
    except ImportError:
        logger.warning(
            "NM_MODELS_FORECASTER_TORCH_PATH set but torch not installed — ignoring PyTorch forecaster"
        )
        return None, None, None
    except OSError as exc:
        logger.warning("PyTorch forecaster not loaded from %s (%s)", p, exc)
        return None, None, None


def _load_forecaster_bundle_if_configured(settings: AppSettings):
    p = settings.models_forecaster_weights_path
    if not p or not Path(p).is_file():
        return None
    try:
        conf = settings.models_forecaster_conformal_state_path
        cfg = _forecaster_config_from_env(conf)
        return load_forecaster_weights(p, cfg=cfg)
    except OSError as exc:
        logger.warning("forecaster weights not loaded from %s (%s); using RNG forward", p, exc)
        return None


def _load_policy_system_if_configured(settings: AppSettings) -> PolicySystem | None:
    p = settings.models_policy_mlp_path
    if not p or not Path(p).is_file():
        return None
    try:
        net = PolicyNetwork()
        net.load(p)
        return PolicySystem(policy_algorithm=net)
    except OSError as exc:
        logger.warning("policy MLP weights not loaded from %s (%s); using heuristic actor", p, exc)
        return None

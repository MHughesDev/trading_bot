"""End-to-end decision: regime (packet) → forecast packet → PolicySystem → proposal (before risk).

Canonical path matches `docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD` §5:
features → forecaster (VSN → CNN → multi-res xLSTM → fusion → quantiles) → ForecastPacket →
PolicySystem → RiskEngine contracts.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from decision_engine.manifest_serving import (
    forecaster_artifacts_resolved,
    policy_manifest_path_broken,
    resolve_manifest_serving_settings,
)
from decision_engine.spec_policy_proposal import run_spec_policy_step
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.inference.stub import ohlc_arrays_from_feature_row
from forecaster_model.models.forecaster_weights import load_forecaster_weights
from policy_model.policy.policy_network import PolicyNetwork
from policy_model.system import PolicySystem

logger = logging.getLogger(__name__)


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


def _abstain_no_trade(symbol: str) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None]:
    """Neutral forecast + NO_TRADE when per-asset guards refuse to serve (FB-AP-003 / FB-AP-004)."""
    regime = RegimeOutput(
        state_index=0,
        semantic=SemanticRegime.SIDEWAYS,
        probabilities=[1.0, 0.0, 0.0, 0.0],
        confidence=0.0,
    )
    fc = ForecastOutput(
        returns_1=0.0,
        returns_3=0.0,
        returns_5=0.0,
        returns_15=0.0,
        volatility=0.0,
        uncertainty=1.0,
    )
    route = RouteDecision(route_id=RouteId.NO_TRADE, confidence=0.0, ranking=[])
    _ = symbol  # reserved for future diagnostics
    return regime, fc, route, None


def _serving_cache_key(settings: AppSettings) -> str:
    return "|".join(
        [
            settings.models_forecaster_torch_path or "",
            settings.models_forecaster_weights_path or "",
            settings.models_forecaster_conformal_state_path or "",
            settings.models_policy_mlp_path or "",
        ]
    )


@dataclass
class _ServingComponents:
    forecaster_weight_bundle: object | None
    torch_model: object | None
    torch_device: object | None
    torch_cfg: object | None
    policy_system: PolicySystem | None


def _build_serving_components_cached(settings: AppSettings) -> _ServingComponents:
    """Load torch + NPZ + policy once per distinct resolved path set (avoid triple torch load)."""
    torch_m, torch_d, torch_c = _load_torch_forecaster_if_configured(settings)
    return _ServingComponents(
        forecaster_weight_bundle=_load_forecaster_bundle_if_configured(settings),
        torch_model=torch_m,
        torch_device=torch_d,
        torch_cfg=torch_c,
        policy_system=_load_policy_system_if_configured(settings),
    )


class DecisionPipeline:
    def __init__(self, settings: AppSettings | None = None) -> None:
        self._settings = settings or load_settings()
        self._last_forecast_packet: ForecastPacket | None = None
        self._serving_cache: dict[str, _ServingComponents] = {}
        self._serving_mode_logged_keys: set[str] = set()

    def _log_serving_mode_once(self, settings: AppSettings, cache_key: str) -> None:
        if cache_key in self._serving_mode_logged_keys:
            return
        self._serving_mode_logged_keys.add(cache_key)
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

    def _get_or_create_serving_components(self, settings: AppSettings) -> _ServingComponents:
        key = _serving_cache_key(settings)
        if key not in self._serving_cache:
            self._serving_cache[key] = _build_serving_components_cached(settings)
            self._log_serving_mode_once(settings, key)
        return self._serving_cache[key]

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

        effective_settings, manifest = resolve_manifest_serving_settings(self._settings, symbol)
        if manifest is not None:
            mid = manifest.runtime_instance_id or manifest.canonical_symbol
            if not forecaster_artifacts_resolved(effective_settings):
                logger.error(
                    "per-asset model manifest %s: no resolvable forecaster torch/NPZ path for symbol %s; "
                    "refusing global fallback — abstaining",
                    mid,
                    symbol,
                )
                self._last_forecast_packet = None
                return _abstain_no_trade(symbol)
            if policy_manifest_path_broken(effective_settings, manifest):
                logger.error(
                    "per-asset model manifest %s: policy_mlp_path %r missing on disk for symbol %s; "
                    "abstaining",
                    mid,
                    manifest.policy_mlp_path,
                    symbol,
                )
                self._last_forecast_packet = None
                return _abstain_no_trade(symbol)

        components = self._get_or_create_serving_components(effective_settings)

        conf_path = effective_settings.models_forecaster_conformal_state_path
        base_cfg = _forecaster_config_from_env(conf_path)
        if components.torch_model is not None and components.torch_cfg is not None:
            cfg = components.torch_cfg
            if conf_path and Path(conf_path).is_file():
                cfg.calibration_enabled = True
        else:
            cfg = base_cfg
        bar_sec = max(1, int(effective_settings.market_data_bar_interval_seconds))
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
            weight_bundle=components.forecaster_weight_bundle if components.torch_model is None else None,
            torch_model=components.torch_model,
            torch_device=components.torch_device,
        )
        pkt.forecast_diagnostics["symbol"] = symbol
        pkt.forecast_diagnostics["pipeline"] = "master_spec"
        pkt.packet_schema_version = 1
        cid = effective_settings.models_forecaster_checkpoint_id
        pkt.source_checkpoint_id = cid
        self._last_forecast_packet = pkt

        regime_out = _regime_output_from_packet(pkt)
        mp = float(mid_price) if mid_price is not None else float(feature_row.get("close", 1.0))
        eq = float(portfolio_equity_usd) if portfolio_equity_usd is not None else 100_000.0
        fc, route, action = run_spec_policy_step(
            symbol,
            pkt,
            settings=effective_settings,
            app_risk=risk,
            mid_price=mp,
            spread_bps=spread_bps,
            portfolio_equity_usd=eq,
            position_signed_qty=position_signed_qty,
            policy_system=components.policy_system,
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

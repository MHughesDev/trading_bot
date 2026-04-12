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
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from app.runtime import asset_model_registry as asset_registry
from decision_engine.spec_policy_proposal import run_spec_policy_step
from forecaster_model.config import ForecasterConfig
from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.inference.stub import ohlc_arrays_from_feature_row
from forecaster_model.models.forecaster_weights import ForecasterWeightBundle, load_forecaster_weights
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


def _abstain_outputs() -> tuple[RegimeOutput, ForecastOutput, RouteDecision, None]:
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
    return regime, fc, route, None


def _forecaster_config_from_env(conformal_path: str | None) -> ForecasterConfig:
    cfg = ForecasterConfig()
    if conformal_path and Path(conformal_path).is_file():
        cfg.calibration_enabled = True
    return cfg


@dataclass(frozen=True)
class _PerSymbolServing:
    """Loaded artifacts for one symbol (manifest mode — FB-AP-003 / FB-AP-004)."""

    forecaster_weight_bundle: ForecasterWeightBundle | None
    torch_model: object | None
    torch_device: object | None
    torch_cfg: ForecasterConfig | None
    policy_system: PolicySystem | None
    conformal_state_path: str | None


class DecisionPipeline:
    def __init__(self, settings: AppSettings | None = None) -> None:
        self._settings = settings or load_settings()
        self._last_forecast_packet: ForecastPacket | None = None
        self._manifest_mode = self._settings.models_use_asset_manifest_paths
        self._manifest_serving_cache: dict[str, _PerSymbolServing | None] = {}
        if self._manifest_mode:
            self._forecaster_weight_bundle = None
            self._torch_model = None
            self._torch_device = None
            self._torch_cfg = None
            self._policy_system = None
        else:
            self._forecaster_weight_bundle = _load_forecaster_bundle_if_configured(self._settings)
            self._torch_model, self._torch_device, self._torch_cfg = _load_torch_forecaster_if_configured(
                self._settings
            )
            self._policy_system = _load_policy_system_if_configured(self._settings)

    @staticmethod
    def _log_serving_mode_once(settings: AppSettings) -> None:
        global _serving_mode_logged
        if _serving_mode_logged:
            return
        _serving_mode_logged = True
        if settings.models_use_asset_manifest_paths:
            logger.info(
                "decision pipeline serving mode: per-asset manifest paths "
                "(NM_MODELS_USE_ASSET_MANIFEST_PATHS=true); global NM_MODELS_* forecaster/policy "
                "paths are not used for inference — load from registry per symbol"
            )
            return
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

    def _resolve_manifest_serving(self, symbol: str) -> _PerSymbolServing | None:
        """Load (and cache) forecaster + policy artifacts from the per-asset manifest only."""
        if symbol in self._manifest_serving_cache:
            return self._manifest_serving_cache[symbol]

        try:
            m = asset_registry.load_manifest(symbol)
        except ValueError as exc:
            logger.error(
                "per-asset manifest mode: invalid manifest for symbol=%s (%s) — abstaining (FB-AP-003)",
                symbol,
                exc,
            )
            self._manifest_serving_cache[symbol] = None
            return None
        if m is None:
            logger.error(
                "per-asset manifest mode: no registry manifest for symbol=%s — abstaining (FB-AP-003)",
                symbol,
            )
            self._manifest_serving_cache[symbol] = None
            return None

        asset_registry.validate_manifest_symbol(symbol, m)
        serving = _load_serving_from_manifest(m)
        self._manifest_serving_cache[symbol] = serving
        return serving

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

        if self._manifest_mode:
            serving = self._resolve_manifest_serving(symbol)
            if serving is None:
                return _abstain_outputs()
            forecaster_bundle = serving.forecaster_weight_bundle
            torch_model = serving.torch_model
            torch_device = serving.torch_device
            torch_cfg = serving.torch_cfg
            policy_system = serving.policy_system
            conf_path = serving.conformal_state_path
        else:
            forecaster_bundle = self._forecaster_weight_bundle
            torch_model = self._torch_model
            torch_device = self._torch_device
            torch_cfg = self._torch_cfg
            policy_system = self._policy_system
            conf_path = self._settings.models_forecaster_conformal_state_path

        base_cfg = _forecaster_config_from_env(conf_path)
        if torch_model is not None and torch_cfg is not None:
            cfg = torch_cfg
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
            weight_bundle=forecaster_bundle if torch_model is None else None,
            torch_model=torch_model,
            torch_device=torch_device,
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
            policy_system=policy_system,
        )
        return regime_out, fc, route, action


def _load_torch_forecaster_at_path(torch_path: str | None):
    if not torch_path or not Path(torch_path).is_file():
        return None, None, None
    try:
        from forecaster_model.inference.torch_infer import load_torch_forecaster_checkpoint

        model, dev, tcfg = load_torch_forecaster_checkpoint(torch_path)
        return model, dev, tcfg
    except ImportError:
        logger.warning(
            "forecaster torch path set but torch not installed — ignoring PyTorch forecaster (%s)",
            torch_path,
        )
        return None, None, None
    except OSError as exc:
        logger.warning("PyTorch forecaster not loaded from %s (%s)", torch_path, exc)
        return None, None, None


def _load_torch_forecaster_if_configured(settings: AppSettings):
    return _load_torch_forecaster_at_path(settings.models_forecaster_torch_path)


def _load_forecaster_bundle_at_path(
    weights_path: str | None,
    conformal_path: str | None,
) -> ForecasterWeightBundle | None:
    if not weights_path or not Path(weights_path).is_file():
        return None
    try:
        cfg = _forecaster_config_from_env(conformal_path)
        return load_forecaster_weights(weights_path, cfg=cfg)
    except OSError as exc:
        logger.warning(
            "forecaster weights not loaded from %s (%s); using RNG forward",
            weights_path,
            exc,
        )
        return None


def _load_forecaster_bundle_if_configured(settings: AppSettings):
    return _load_forecaster_bundle_at_path(
        settings.models_forecaster_weights_path,
        settings.models_forecaster_conformal_state_path,
    )


def _load_policy_system_at_path(policy_path: str | None) -> PolicySystem | None:
    if not policy_path or not Path(policy_path).is_file():
        return None
    try:
        net = PolicyNetwork()
        net.load(policy_path)
        return PolicySystem(policy_algorithm=net)
    except OSError as exc:
        logger.warning(
            "policy MLP weights not loaded from %s (%s); using heuristic actor",
            policy_path,
            exc,
        )
        return None


def _load_policy_system_if_configured(settings: AppSettings) -> PolicySystem | None:
    return _load_policy_system_at_path(settings.models_policy_mlp_path)


def _load_serving_from_manifest(manifest: AssetModelManifest) -> _PerSymbolServing | None:
    """
    Forecaster and policy paths come **only** from the manifest so we never apply
    global `NM_MODELS_*` weights to a different symbol (FB-AP-003, FB-AP-004).
    """
    sym = manifest.canonical_symbol
    torch_p = manifest.forecaster_torch_path
    npz_p = manifest.forecaster_weights_path
    conf_p = manifest.forecaster_conformal_state_path
    policy_p = manifest.policy_mlp_path

    torch_ok = bool(torch_p and Path(torch_p).is_file())
    npz_ok = bool(npz_p and Path(npz_p).is_file())

    if torch_p and not torch_ok:
        logger.error(
            "per-asset manifest for %s: forecaster_torch_path %r not on disk — abstaining (FB-AP-003)",
            sym,
            torch_p,
        )
        return None
    if not torch_ok and npz_p and not npz_ok:
        logger.error(
            "per-asset manifest for %s: forecaster_weights_path %r not on disk — abstaining (FB-AP-003)",
            sym,
            npz_p,
        )
        return None
    if not torch_ok and not npz_ok:
        logger.error(
            "per-asset manifest for %s: no forecaster_torch_path or forecaster_weights_path "
            "— abstaining (FB-AP-003)",
            sym,
        )
        return None

    torch_model, torch_device, torch_cfg = _load_torch_forecaster_at_path(torch_p if torch_ok else None)

    bundle: ForecasterWeightBundle | None = None
    if torch_model is None and npz_ok:
        bundle = _load_forecaster_bundle_at_path(npz_p, conf_p)

    if torch_model is None and bundle is None:
        logger.error(
            "per-asset manifest for %s: could not load forecaster weights — abstaining (FB-AP-003)",
            sym,
        )
        return None

    if policy_p and not Path(policy_p).is_file():
        logger.error(
            "per-asset manifest for %s: policy_mlp_path %r missing on disk — abstaining (FB-AP-004)",
            sym,
            policy_p,
        )
        return None

    pol = _load_policy_system_at_path(policy_p)

    return _PerSymbolServing(
        forecaster_weight_bundle=bundle,
        torch_model=torch_model,
        torch_device=torch_device,
        torch_cfg=torch_cfg,
        policy_system=pol,
        conformal_state_path=conf_p if conf_p and Path(conf_p).is_file() else None,
    )

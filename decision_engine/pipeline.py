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
from app.runtime.asset_model_registry import load_manifest
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


def _forecaster_config_from_env(conformal_path: str | None) -> ForecasterConfig:
    cfg = ForecasterConfig()
    if conformal_path and Path(conformal_path).is_file():
        cfg.calibration_enabled = True
    return cfg


def _file_exists(p: str | None) -> bool:
    return bool(p and Path(p).is_file())


def _global_model_paths_configured(settings: AppSettings) -> bool:
    return (
        _file_exists(settings.models_forecaster_torch_path)
        or _file_exists(settings.models_forecaster_weights_path)
        or _file_exists(settings.models_policy_mlp_path)
    )


def _multi_symbol(settings: AppSettings) -> bool:
    return len(settings.market_data_symbols) > 1


@dataclass(frozen=True)
class _ResolvedPaths:
    """Per-tick serving paths after manifest + FB-AP-003 / FB-AP-004 binding rules."""

    forecaster_torch_path: str | None
    forecaster_weights_path: str | None
    forecaster_conformal_state_path: str | None
    policy_mlp_path: str | None
    forecaster_checkpoint_id: str | None
    manifest: AssetModelManifest | None
    binding_abstain: bool
    binding_reason: str | None


def _paths_from_manifest_only(m: AssetModelManifest) -> _ResolvedPaths:
    """Multi-asset + global NM_MODELS_* set: only manifest fields apply (no global fallback)."""
    pol = m.policy_mlp_path or m.policy_checkpoint_path
    return _ResolvedPaths(
        forecaster_torch_path=m.forecaster_torch_path,
        forecaster_weights_path=m.forecaster_weights_path,
        forecaster_conformal_state_path=m.forecaster_conformal_state_path,
        policy_mlp_path=pol,
        forecaster_checkpoint_id=None,
        manifest=m,
        binding_abstain=False,
        binding_reason=None,
    )


def _merge_manifest_and_settings(
    manifest: AssetModelManifest | None, settings: AppSettings
) -> _ResolvedPaths:
    """Single-symbol or multi-symbol without global file paths: manifest overrides when present."""
    if manifest is None:
        return _ResolvedPaths(
            forecaster_torch_path=settings.models_forecaster_torch_path,
            forecaster_weights_path=settings.models_forecaster_weights_path,
            forecaster_conformal_state_path=settings.models_forecaster_conformal_state_path,
            policy_mlp_path=settings.models_policy_mlp_path,
            forecaster_checkpoint_id=settings.models_forecaster_checkpoint_id,
            manifest=None,
            binding_abstain=False,
            binding_reason=None,
        )
    m = manifest
    pol = m.policy_mlp_path or m.policy_checkpoint_path
    return _ResolvedPaths(
        forecaster_torch_path=m.forecaster_torch_path or settings.models_forecaster_torch_path,
        forecaster_weights_path=m.forecaster_weights_path or settings.models_forecaster_weights_path,
        forecaster_conformal_state_path=m.forecaster_conformal_state_path
        or settings.models_forecaster_conformal_state_path,
        policy_mlp_path=pol or settings.models_policy_mlp_path,
        forecaster_checkpoint_id=settings.models_forecaster_checkpoint_id,
        manifest=m,
        binding_abstain=False,
        binding_reason=None,
    )


def resolve_serving_paths(symbol: str, settings: AppSettings) -> _ResolvedPaths:
    """
    FB-AP-003 / FB-AP-004: bind forecaster and policy artifacts to the decision symbol.

    When multiple symbols are configured and global ``NM_MODELS_*`` file paths are set, refuse to
    apply those files to any symbol without a per-asset manifest (prevents one checkpoint serving
    every asset). When a manifest exists, only manifest paths are used in that mode (no global
    fallback for model files).
    """
    sym = symbol.strip()
    manifest = load_manifest(sym)
    multi = _multi_symbol(settings)
    global_files = _global_model_paths_configured(settings)

    if multi and global_files:
        if manifest is None:
            logger.error(
                "FB-AP-003/004 forecaster/policy binding refused: multi-symbol settings with global "
                "model file paths require a per-asset manifest for symbol=%r (no manifest on disk). "
                "Refusing global NM_MODELS_* weights for this tick.",
                sym,
            )
            return _ResolvedPaths(
                forecaster_torch_path=None,
                forecaster_weights_path=None,
                forecaster_conformal_state_path=None,
                policy_mlp_path=None,
                forecaster_checkpoint_id=settings.models_forecaster_checkpoint_id,
                manifest=None,
                binding_abstain=True,
                binding_reason=(
                    f"multi-symbol mode: global model paths are set but no manifest for {sym!r}"
                ),
            )
        mp = _paths_from_manifest_only(manifest)
        return _ResolvedPaths(
            forecaster_torch_path=mp.forecaster_torch_path,
            forecaster_weights_path=mp.forecaster_weights_path,
            forecaster_conformal_state_path=mp.forecaster_conformal_state_path,
            policy_mlp_path=mp.policy_mlp_path,
            forecaster_checkpoint_id=settings.models_forecaster_checkpoint_id,
            manifest=manifest,
            binding_abstain=False,
            binding_reason=None,
        )

    return _merge_manifest_and_settings(manifest, settings)


def _resolved_paths_key(rp: _ResolvedPaths) -> tuple:
    return (
        rp.forecaster_torch_path,
        rp.forecaster_weights_path,
        rp.forecaster_conformal_state_path,
        rp.policy_mlp_path,
    )


class DecisionPipeline:
    def __init__(self, settings: AppSettings | None = None) -> None:
        self._settings = settings or load_settings()
        self._last_forecast_packet: ForecastPacket | None = None
        self._cache_key: tuple | None = None
        self._forecaster_weight_bundle: ForecasterWeightBundle | None = None
        self._torch_model = None
        self._torch_device = None
        self._torch_cfg: ForecasterConfig | None = None
        self._policy_system: PolicySystem | None = None

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

    def _ensure_artifacts(self, rp: _ResolvedPaths) -> None:
        key = _resolved_paths_key(rp)
        if key == self._cache_key:
            return
        self._cache_key = key
        settings = self._settings
        eff = settings.model_copy(
            update={
                "models_forecaster_torch_path": rp.forecaster_torch_path,
                "models_forecaster_weights_path": rp.forecaster_weights_path,
                "models_forecaster_conformal_state_path": rp.forecaster_conformal_state_path,
                "models_policy_mlp_path": rp.policy_mlp_path,
            }
        )
        self._forecaster_weight_bundle = _load_forecaster_bundle_if_configured(eff)
        self._torch_model, self._torch_device, self._torch_cfg = _load_torch_forecaster_if_configured(
            eff
        )
        self._policy_system = _load_policy_system_if_configured(eff)

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

        rp = resolve_serving_paths(symbol, self._settings)
        self._ensure_artifacts(rp)

        conf_path = rp.forecaster_conformal_state_path
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
        pkt.packet_schema_version = 1
        cid = rp.forecaster_checkpoint_id
        pkt.source_checkpoint_id = cid
        if rp.manifest is not None:
            pkt.forecast_diagnostics["asset_model_manifest_id"] = rp.manifest.canonical_symbol
            if rp.manifest.runtime_instance_id:
                pkt.forecast_diagnostics["asset_model_runtime_instance_id"] = (
                    rp.manifest.runtime_instance_id
                )
        if rp.binding_abstain and rp.binding_reason:
            pkt.forecast_diagnostics["binding_abstain"] = True
            pkt.forecast_diagnostics["binding_reason"] = rp.binding_reason
        self._last_forecast_packet = pkt

        regime_out = _regime_output_from_packet(pkt)
        mp = float(mid_price) if mid_price is not None else float(feature_row.get("close", 1.0))
        eq = float(portfolio_equity_usd) if portfolio_equity_usd is not None else 100_000.0

        if rp.binding_abstain:
            fc = ForecastOutput(
                returns_1=0.0,
                returns_3=0.0,
                returns_5=0.0,
                returns_15=0.0,
                volatility=0.0,
                uncertainty=1.0,
            )
            route = RouteDecision(
                route_id=RouteId.NO_TRADE,
                confidence=0.0,
                ranking=[RouteId.NO_TRADE],
            )
            return regime_out, fc, route, None

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

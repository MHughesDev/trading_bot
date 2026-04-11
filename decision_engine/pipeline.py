"""End-to-end decision: regime → forecast → route → action proposal (before risk)."""

from __future__ import annotations

import logging
from decimal import Decimal
from pathlib import Path

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState
from decision_engine.action_generator import propose_action
from decision_engine.forecast_packet_adapter import forecast_packet_to_forecast_output
from decision_engine.spec_policy_proposal import run_spec_policy_step
from forecaster_model.inference.stub import build_forecast_packet_stub, ohlc_arrays_from_feature_row
from models.forecast.tft_forecast import TemporalFusionForecaster
from models.regime.hmm_regime import GaussianHMMRegimeModel
from models.routing.route_selector import DeterministicRouteSelector

logger = logging.getLogger(__name__)


def _feature_vector(values: dict[str, float], dim: int = 32) -> np.ndarray:
    vec = np.zeros(dim, dtype=np.float64)
    keys = sorted(values.keys())
    for i, k in enumerate(keys[:dim]):
        vec[i] = float(values[k])
    return vec


def _load_regime(settings: AppSettings) -> GaussianHMMRegimeModel:
    p = settings.models_regime_path
    if p and Path(p).is_file():
        try:
            return GaussianHMMRegimeModel.load(p)
        except Exception:
            logger.exception("failed to load regime model from %s; using bootstrap HMM", p)
    return GaussianHMMRegimeModel()


def _load_forecast(settings: AppSettings) -> TemporalFusionForecaster:
    p = settings.models_forecast_path
    if p and Path(p).is_file():
        try:
            return TemporalFusionForecaster.load(p)
        except Exception:
            logger.exception("failed to load forecast model from %s; using Ridge bootstrap", p)
    return TemporalFusionForecaster()


class DecisionPipeline:
    def __init__(
        self,
        regime_model: GaussianHMMRegimeModel | None = None,
        forecaster: TemporalFusionForecaster | None = None,
        router: DeterministicRouteSelector | None = None,
        settings: AppSettings | None = None,
    ) -> None:
        self._settings = settings or load_settings()
        self.regime = regime_model or _load_regime(self._settings)
        self.forecast = forecaster or _load_forecast(self._settings)
        self.router = router or DeterministicRouteSelector(self._settings)
        self._last_forecast_packet: ForecastPacket | None = None

    @property
    def last_forecast_packet(self) -> ForecastPacket | None:
        """Set when a packet is built: diagnostics flag and/or `decision_forecast_routing_source=packet` (FB-FR-PG1)."""
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
        X = _feature_vector(feature_row).reshape(1, -1)
        regime_out = self.regime.predict_proba_last(X)
        fc_ridge = self.forecast.predict(_feature_vector(feature_row))

        self._last_forecast_packet = None
        pkt: ForecastPacket | None = None
        spec_mode = self._settings.decision_pipeline_mode == "spec_policy"
        need_packet = (
            spec_mode
            or self._settings.decision_forecast_packet_enabled
            or (self._settings.decision_forecast_routing_source == "packet")
        )
        if need_packet:
            o, h, lo, cl, vo = ohlc_arrays_from_feature_row(feature_row)
            pkt = build_forecast_packet_stub(o, h, lo, cl, vo)
            pkt.forecast_diagnostics["symbol"] = symbol
            pkt.forecast_diagnostics["routing_source"] = self._settings.decision_forecast_routing_source
            pkt.forecast_diagnostics["pipeline_mode"] = self._settings.decision_pipeline_mode
            self._last_forecast_packet = pkt

        if spec_mode:
            assert pkt is not None
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

        if self._settings.decision_forecast_routing_source == "packet":
            assert pkt is not None
            fc = forecast_packet_to_forecast_output(pkt)
        else:
            fc = fc_ridge

        route = self.router.decide(symbol, fc, regime_out, spread_bps, risk)
        action = propose_action(symbol, route.route_id, fc)
        return regime_out, fc, route, action

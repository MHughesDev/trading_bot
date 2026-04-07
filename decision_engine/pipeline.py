"""End-to-end decision: regime → forecast → route → action proposal (before risk)."""

from __future__ import annotations

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import ActionProposal, RouteDecision
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput
from app.contracts.risk import RiskState
from decision_engine.action_generator import propose_action
from models.forecast.tft_forecast import TemporalFusionForecaster
from models.regime.hmm_regime import GaussianHMMRegimeModel
from models.routing.route_selector import DeterministicRouteSelector


def _feature_vector(values: dict[str, float], dim: int = 32) -> np.ndarray:
    vec = np.zeros(dim, dtype=np.float64)
    keys = sorted(values.keys())
    for i, k in enumerate(keys[:dim]):
        vec[i] = float(values[k])
    return vec


class DecisionPipeline:
    def __init__(
        self,
        regime_model: GaussianHMMRegimeModel | None = None,
        forecaster: TemporalFusionForecaster | None = None,
        router: DeterministicRouteSelector | None = None,
        settings: AppSettings | None = None,
    ) -> None:
        self._settings = settings or load_settings()
        self.regime = regime_model or GaussianHMMRegimeModel()
        self.forecast = forecaster or TemporalFusionForecaster()
        self.router = router or DeterministicRouteSelector(self._settings)

    def step(
        self,
        symbol: str,
        feature_row: dict[str, float],
        spread_bps: float,
        risk: RiskState,
    ) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None]:
        X = _feature_vector(feature_row).reshape(1, -1)
        regime_out = self.regime.predict_proba_last(X)
        fc = self.forecast.predict(_feature_vector(feature_row))
        route = self.router.decide(symbol, fc, regime_out, spread_bps, risk)
        action = propose_action(symbol, route.route_id, fc)
        return regime_out, fc, route, action

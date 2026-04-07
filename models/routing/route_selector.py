"""Deterministic route selection: forecast strength, regime fit, spread/liquidity, risk mode."""

from __future__ import annotations

from app.config.settings import AppSettings, load_settings
from app.contracts.decisions import RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState, SystemMode


class DeterministicRouteSelector:
    """Thresholds from `routing` in default.yaml / NM_ROUTING_* env."""

    def __init__(self, settings: AppSettings | None = None) -> None:
        self._settings = settings or load_settings()

    def decide(
        self,
        _symbol: str,
        forecast: ForecastOutput,
        regime: RegimeOutput,
        spread_bps: float,
        risk: RiskState,
    ) -> RouteDecision:
        if risk.mode != SystemMode.RUNNING:
            return RouteDecision(route_id=RouteId.NO_TRADE, confidence=1.0, ranking=[RouteId.NO_TRADE])

        s = self._settings
        strength = abs(forecast.returns_5) + abs(forecast.returns_15) * 0.5
        if spread_bps > s.routing_spread_trade_max_bps or strength < s.routing_forecast_strength_min:
            return RouteDecision(
                route_id=RouteId.NO_TRADE,
                confidence=0.7,
                ranking=[RouteId.NO_TRADE, RouteId.SCALPING, RouteId.INTRADAY, RouteId.SWING],
            )

        pen = s.routing_spread_penalty_per_bp
        spread_term = spread_bps * pen
        scores: dict[RouteId, float] = {
            RouteId.SCALPING: strength * s.routing_score_scalping_forecast - spread_term,
            RouteId.INTRADAY: strength * s.routing_score_intraday_forecast
            + self._regime_bonus(regime.semantic, RouteId.INTRADAY)
            - spread_term,
            RouteId.SWING: abs(forecast.returns_15) * s.routing_score_swing_forecast
            + self._regime_bonus(regime.semantic, RouteId.SWING)
            - spread_term,
            RouteId.NO_TRADE: 0.0,
        }
        ranked = sorted(scores.keys(), key=lambda r: scores[r], reverse=True)
        best = ranked[0]
        conf = min(1.0, max(0.0, scores[best] / (scores[ranked[1]] + 1e-6 + scores[best])))
        return RouteDecision(route_id=best, confidence=conf, ranking=ranked)

    @staticmethod
    def _regime_bonus(sem: SemanticRegime, route: RouteId) -> float:
        if route == RouteId.SWING and sem in (SemanticRegime.BULL, SemanticRegime.BEAR):
            return 0.05
        if route == RouteId.INTRADAY and sem == SemanticRegime.VOLATILE:
            return 0.05
        if route == RouteId.SCALPING and sem == SemanticRegime.VOLATILE:
            return 0.08
        return 0.0

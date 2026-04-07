from __future__ import annotations

from dataclasses import dataclass

from app.contracts.common import RouteId, SemanticRegime
from app.contracts.decisions import RouteDecision
from app.contracts.models import ForecastOutput, RegimeOutput, RouteScore


@dataclass(slots=True)
class DeterministicRouteSelector:
    """
    V1 deterministic route scoring model.

    Inputs:
    - forecast strength/confidence
    - regime compatibility
    - spread/liquidity proxy
    - risk pressure proxy
    """

    no_trade_threshold: float = 0.15

    def select(
        self,
        forecast: ForecastOutput,
        regime: RegimeOutput,
        spread_bps: float,
        risk_pressure: float,
    ) -> tuple[RouteDecision, list[RouteScore]]:
        # Forecast strength from multi-horizon expected returns.
        expected_abs = sum(abs(v) for v in forecast.horizon_returns.values()) / max(
            len(forecast.horizon_returns), 1
        )
        forecast_strength = min(expected_abs / 0.01, 1.0)

        regime_bonus = self._regime_bonus(regime.semantic_state)
        spread_penalty = min(max(spread_bps, 0.0) / 30.0, 1.0)
        risk_penalty = min(max(risk_pressure, 0.0), 1.0)

        route_scores = [
            RouteScore(
                route_id=RouteId.SCALPING,
                score=self._clip(
                    0.45 * forecast_strength
                    + 0.25 * forecast.confidence
                    + 0.20 * regime_bonus["scalping"]
                    - 0.25 * spread_penalty
                    - 0.15 * risk_penalty
                ),
                components={
                    "forecast_strength": forecast_strength,
                    "forecast_conf": forecast.confidence,
                    "regime_bonus": regime_bonus["scalping"],
                    "spread_penalty": spread_penalty,
                    "risk_penalty": risk_penalty,
                },
            ),
            RouteScore(
                route_id=RouteId.INTRADAY,
                score=self._clip(
                    0.40 * forecast_strength
                    + 0.30 * forecast.confidence
                    + 0.25 * regime_bonus["intraday"]
                    - 0.15 * spread_penalty
                    - 0.20 * risk_penalty
                ),
                components={
                    "forecast_strength": forecast_strength,
                    "forecast_conf": forecast.confidence,
                    "regime_bonus": regime_bonus["intraday"],
                    "spread_penalty": spread_penalty,
                    "risk_penalty": risk_penalty,
                },
            ),
            RouteScore(
                route_id=RouteId.SWING,
                score=self._clip(
                    0.30 * forecast_strength
                    + 0.35 * forecast.confidence
                    + 0.35 * regime_bonus["swing"]
                    - 0.10 * spread_penalty
                    - 0.20 * risk_penalty
                ),
                components={
                    "forecast_strength": forecast_strength,
                    "forecast_conf": forecast.confidence,
                    "regime_bonus": regime_bonus["swing"],
                    "spread_penalty": spread_penalty,
                    "risk_penalty": risk_penalty,
                },
            ),
        ]

        route_scores_sorted = sorted(route_scores, key=lambda r: r.score, reverse=True)
        ranking = [r.route_id for r in route_scores_sorted]
        best = route_scores_sorted[0]

        if best.score < self.no_trade_threshold:
            decision = RouteDecision(
                route_id=RouteId.NO_TRADE,
                confidence=1.0 - best.score,
                ranking=[RouteId.NO_TRADE, *ranking],
                reasons=["best_route_score_below_threshold"],
            )
            return decision, route_scores_sorted

        decision = RouteDecision(
            route_id=best.route_id,
            confidence=best.score,
            ranking=ranking,
            reasons=["deterministic_route_selector_v1"],
        )
        return decision, route_scores_sorted

    def _regime_bonus(self, regime: SemanticRegime) -> dict[str, float]:
        if regime == SemanticRegime.VOLATILE:
            return {"scalping": 0.9, "intraday": 0.6, "swing": 0.2}
        if regime == SemanticRegime.BULL:
            return {"scalping": 0.6, "intraday": 0.8, "swing": 0.9}
        if regime == SemanticRegime.BEAR:
            return {"scalping": 0.7, "intraday": 0.7, "swing": 0.4}
        return {"scalping": 0.5, "intraday": 0.6, "swing": 0.6}

    def _clip(self, value: float) -> float:
        return max(0.0, min(value, 1.0))

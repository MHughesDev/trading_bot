"""Strategy-based live decision source (FB-AP-XXX — replaces the retired AI decision pipeline).

The runtime no longer routes through the forecaster/regime/RL-policy stack
(``decision_engine``/``forecaster_model``/``policy_model`` — preserved for reference under
``legacy/decision_pipeline/``). Instead the loop is as simple as the product wants it: a user
picks an asset and a strategy on the Asset page (:mod:`app.runtime.asset_strategy_selection`
persists the choice), and that strategy *is* the live decision source for the asset — paper or
live, the same engine the "Backtest" panel uses.

Each tick, :func:`run_strategy_decision_tick` re-runs the asset's configured strategy over a
trailing window of recent bars via :func:`backtesting.nautilus_backtest.run_backtest` (the exact
engine/contract the Asset-page backtest panel exposes). The strategy "passes" when it would have
been net profitable in the quote currency over that window; on a pass, the directional signal is
the side of its most recent simulated fill (BUY → long, SELL → short, no fills → flat).

Gating mirrors the old pipeline's real gates — ``product_cache.is_tradable`` is the hard
tradability gate; weekends get a soft size throttle (the system trades crypto 24/7, so this is
a reduction, never a hard close).

Returns the same ``(regime, forecast, route, proposal, trade, risk_state)`` 6-tuple shape the
retired ``run_decision_tick`` returned, so execution / risk / audit / observability downstream
keep working unchanged — the values inside are simply derived from the strategy run rather than
from a forecaster+regime+RL stack.
"""

from __future__ import annotations

import logging
from datetime import UTC, datetime
from decimal import Decimal
from typing import TYPE_CHECKING, Any

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId, TradeAction
from app.contracts.forecast import ForecastOutput
from app.contracts.regime import RegimeOutput, SemanticRegime
from app.contracts.risk import RiskState
from app.runtime import asset_strategy_selection
from backtesting.nautilus_backtest import run_backtest
from strategies.registry import get_strategy

if TYPE_CHECKING:
    import polars as pl

logger = logging.getLogger(__name__)

#: How many recent completed bars to replay the strategy over when checking "would this strategy
#: have been profitable lately" — long enough for the strategy's own indicators to warm up.
TRAILING_WINDOW_BARS = 200
#: Minimum bars required before a strategy is even attempted (avoids noisy single-bar verdicts).
MIN_BARS_FOR_DECISION = 20
#: Soft weekend size throttle — crypto trades 24/7; weekends just get smaller size, never a halt.
WEEKEND_SIZE_THROTTLE = 0.5

_NEUTRAL_FORECAST = ForecastOutput(
    returns_1=0.0, returns_3=0.0, returns_5=0.0, returns_15=0.0, volatility=0.0, uncertainty=1.0,
)
_REGIME_INDEX = {
    SemanticRegime.BULL: 0,
    SemanticRegime.BEAR: 1,
    SemanticRegime.VOLATILE: 2,
    SemanticRegime.SIDEWAYS: 3,
}


def session_size_throttle(data_timestamp: datetime | None) -> tuple[str, float]:
    """``(session_mode, size_throttle)`` — soft weekend throttle; otherwise full size."""
    ts = data_timestamp or datetime.now(UTC)
    ts = ts.replace(tzinfo=UTC) if ts.tzinfo is None else ts.astimezone(UTC)
    if ts.weekday() >= 5:
        return "weekend", WEEKEND_SIZE_THROTTLE
    return "regular", 1.0


def _regime_for_direction(direction: int, *, confidence: float) -> RegimeOutput:
    semantic = (
        SemanticRegime.BULL if direction > 0 else SemanticRegime.BEAR if direction < 0 else SemanticRegime.SIDEWAYS
    )
    idx = _REGIME_INDEX[semantic]
    probabilities = [0.0, 0.0, 0.0, 0.0]
    probabilities[idx] = 1.0
    return RegimeOutput(state_index=idx, semantic=semantic, probabilities=probabilities, confidence=confidence)


def _signal_from_backtest(result: Any, *, quote_currency: str) -> tuple[int, bool]:
    """``(direction, passed)``.

    "Passed" = the strategy's simulated total PnL in ``quote_currency`` over the trailing window
    was positive. The direction is the side of its most recent simulated fill — the strategy's
    own most up-to-date read on where price is headed.
    """
    pnl = result.stats_pnls.get(quote_currency, {}).get("PnL (total)")
    passed = isinstance(pnl, (int, float)) and pnl > 0.0
    if not passed or not result.fills:
        return 0, passed
    side = str(result.fills[-1].get("side", "")).upper()
    if side == "BUY":
        return 1, passed
    if side == "SELL":
        return -1, passed
    return 0, passed


def run_strategy_decision_tick(
    *,
    symbol: str,
    settings: AppSettings,
    risk_state: RiskState,
    risk_engine: Any,
    mid_price: float,
    spread_bps: float,
    data_timestamp: datetime | None,
    product_tradable: bool = True,
    ohlc_history: "pl.DataFrame | None" = None,
    current_total_exposure_usd: float = 0.0,
    feed_last_message_at: datetime | None = None,
    position_signed_qty: Decimal | None = None,
    available_cash_usd: float | None = None,
    portfolio_equity_usd: float | None = None,
) -> tuple[RegimeOutput, ForecastOutput, RouteDecision, ActionProposal | None, TradeAction | None, RiskState]:
    """Strategy-based replacement for the retired ``run_decision_tick``.

    "That simple": looks up the asset's chosen strategy, replays it over recent bars, and trades
    in the direction it just signalled when it's been net profitable lately. Returns the
    ``(regime, forecast, route, proposal, trade, risk_state)`` 6-tuple the execution/risk/audit
    layers already understand.
    """
    session_mode, size_throttle = session_size_throttle(data_timestamp)
    risk_state = risk_state.model_copy(update={"session_mode": session_mode})

    strategy_key = asset_strategy_selection.effective_strategy_for_symbol(symbol, settings)
    direction = 0
    passed = False
    if (
        product_tradable
        and strategy_key is not None
        and get_strategy(strategy_key) is not None
        and ohlc_history is not None
        and ohlc_history.height >= MIN_BARS_FOR_DECISION
    ):
        _, _, quote_code = symbol.partition("-")
        quote_currency = quote_code or "USD"
        try:
            result = run_backtest(
                symbol=symbol,
                strategy_key=strategy_key,
                bars=ohlc_history.tail(TRAILING_WINDOW_BARS).to_dicts(),
                strategy_params=asset_strategy_selection.read_strategy_params(symbol) or None,
                interval_seconds=int(settings.market_data_bar_interval_seconds),
                starting_currency=quote_currency,
            )
            direction, passed = _signal_from_backtest(result, quote_currency=quote_currency)
        except Exception:
            logger.exception(
                "strategy_decision_backtest_failed symbol=%s strategy=%s", symbol, strategy_key,
            )
            direction, passed = 0, False

    regime = _regime_for_direction(direction, confidence=1.0 if passed else 0.0)
    route = RouteDecision(
        route_id=RouteId.SWING if (direction != 0 and passed) else RouteId.NO_TRADE,
        confidence=1.0 if passed else 0.0,
    )

    proposal: ActionProposal | None = None
    if direction != 0 and passed and product_tradable:
        proposal = ActionProposal(
            symbol=symbol,
            route_id=route.route_id,
            direction=direction,
            size_fraction=max(0.0, min(1.0, size_throttle)),
            stop_distance_pct=0.0,
        )

    trade, risk_state = risk_engine.evaluate(
        symbol,
        proposal,
        risk_state,
        mid_price=mid_price,
        spread_bps=spread_bps,
        data_timestamp=data_timestamp,
        current_total_exposure_usd=current_total_exposure_usd,
        feed_last_message_at=feed_last_message_at,
        product_tradable=product_tradable,
        position_signed_qty=position_signed_qty,
        available_cash_usd=available_cash_usd,
        portfolio_equity_usd=portfolio_equity_usd,
    )
    return regime, _NEUTRAL_FORECAST, route, proposal, trade, risk_state

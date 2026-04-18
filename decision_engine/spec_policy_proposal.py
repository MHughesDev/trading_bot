"""Map PolicySystem output + ForecastPacket routing adapter → legacy contracts for RiskEngine."""

from __future__ import annotations

from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.decisions import ActionProposal, RouteDecision, RouteId
from app.contracts.forecast import ForecastOutput
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState as AppRiskState
from app.contracts.canonical_state import CanonicalStateOutput
from app.contracts.canonical_structure import CanonicalStructureOutput
from app.contracts.trigger import TriggerOutput
from decision_engine.auction_engine import run_opportunity_auction
from decision_engine.forecast_packet_adapter import forecast_packet_to_forecast_output
from policy_model.bridge import policy_envelope_from_app_settings
from policy_model.objects import ExecutionState, PortfolioState, RiskState as PolicyRiskState
from policy_model.system import PolicySystem

# Maps |required_delta_fraction| ∈ [0,1] to RiskEngine `size_fraction` ∈ (0,1].
# Chosen so a full delta to max position maps to size_fraction=1; aligns with prior
# per-route slot fractions (FB-CAN-019 removed standalone `action_generator.py`).
SPEC_POLICY_DELTA_TO_SIZE_FRACTION_SCALE = 4.0


def build_portfolio_state_for_spec(
    *,
    equity_usd: float,
    mid_price: float,
    position_signed_qty: Decimal | None,
) -> PortfolioState:
    """Construct `PortfolioState` from runtime equity and venue position."""
    eq = max(float(equity_usd), 1e-9)
    qty = float(position_signed_qty) if position_signed_qty is not None else 0.0
    signed_notional = qty * float(mid_price)
    pos_frac = signed_notional / eq
    cash = max(0.0, eq - abs(signed_notional))
    return PortfolioState(
        equity=eq,
        cash=cash,
        position_units=qty,
        position_notional=abs(signed_notional),
        position_fraction=pos_frac,
        entry_price=float(mid_price) if qty != 0 else None,
        unrealized_pnl=0.0,
        realized_pnl=0.0,
        current_leverage=1.0,
        time_in_position=0,
        last_action=None,
        last_trade_timestamp=None,
    )


def build_execution_state(mid_price: float, spread_bps: float, settings: AppSettings) -> ExecutionState:
    slip = settings.backtesting_slippage_bps / 10_000.0 * mid_price
    fee = settings.backtesting_fee_bps / 10_000.0
    return ExecutionState(
        mid_price=float(mid_price),
        spread=spread_bps / 10_000.0 * mid_price,
        estimated_slippage=slip,
        estimated_fee_rate=fee,
        available_liquidity_score=1.0,
        latency_proxy=0.01,
        volatility_proxy=0.02,
    )


def execution_plan_to_proposal(
    symbol: str,
    *,
    forecast_packet: ForecastPacket,
    fc: ForecastOutput,
    plan,
) -> ActionProposal | None:
    """Turn `ExecutionPlan` into `ActionProposal` for `RiskEngine` (spec pipeline mode)."""
    if plan.skip_execution:
        return None
    delta = plan.required_delta_fraction
    if abs(delta) < 1e-12:
        return None
    direction = 1 if delta > 0 else -1 if delta < 0 else 0
    if direction == 0:
        return None
    size_fraction = min(1.0, abs(delta) * SPEC_POLICY_DELTA_TO_SIZE_FRACTION_SCALE)
    if size_fraction <= 0:
        return None
    stop_pct = max(0.002, min(0.05, float(fc.volatility) * 2.0))
    return ActionProposal(
        symbol=symbol,
        route_id=RouteId.INTRADAY,
        direction=direction,
        size_fraction=size_fraction,
        stop_distance_pct=stop_pct,
        order_type="market",
        expiry_seconds=900,
    )


def run_spec_policy_step(
    symbol: str,
    forecast_packet: ForecastPacket,
    *,
    settings: AppSettings,
    app_risk: AppRiskState,
    mid_price: float,
    spread_bps: float,
    portfolio_equity_usd: float,
    position_signed_qty: Decimal | None,
    policy_system: PolicySystem | None = None,
    trigger: TriggerOutput | None = None,
    apex: CanonicalStateOutput | None = None,
    feature_row: dict[str, float] | None = None,
    structure: CanonicalStructureOutput | None = None,
) -> tuple[ForecastOutput, RouteDecision, ActionProposal | None]:
    """
    Human-spec path: PolicySystem + ExecutionPlan → proposal; `ForecastOutput` from packet for metrics.
    """
    fc = forecast_packet_to_forecast_output(forecast_packet)
    ps = build_portfolio_state_for_spec(
        equity_usd=portfolio_equity_usd,
        mid_price=mid_price,
        position_signed_qty=position_signed_qty,
    )
    es = build_execution_state(mid_price, spread_bps, settings)
    env_risk: PolicyRiskState = policy_envelope_from_app_settings(settings, app_risk)
    sys = policy_system or PolicySystem()
    out = sys.decide(forecast_packet, ps, es, env_risk)
    plan = out["execution_plan"]
    proposal = execution_plan_to_proposal(symbol, forecast_packet=forecast_packet, fc=fc, plan=plan)

    if trigger is not None:
        forecast_packet.forecast_diagnostics["trigger"] = trigger.model_dump()
        if not trigger.trigger_valid:
            proposal = None
        elif proposal is not None:
            scale = 0.2 + 0.8 * float(trigger.trigger_confidence)
            proposal = proposal.model_copy(
                update={"size_fraction": min(1.0, proposal.size_fraction * scale)}
            )

    if apex is not None and trigger is not None and feature_row is not None:
        try:
            top_n_auction = int(settings.canonical.domains.auction.get("top_n", 1))
        except Exception:
            top_n_auction = 1
        proposal, auction_result = run_opportunity_auction(
            symbol,
            forecast_packet,
            apex=apex,
            trigger=trigger,
            app_risk=app_risk,
            spread_bps=spread_bps,
            feature_row=feature_row,
            settings=settings,
            portfolio_equity_usd=portfolio_equity_usd,
            position_signed_qty=position_signed_qty,
            base_proposal=proposal,
            top_n=top_n_auction,
            structure=structure,
        )
        forecast_packet.forecast_diagnostics["auction"] = auction_result.model_dump()

    if proposal is None:
        route = RouteDecision(
            route_id=RouteId.NO_TRADE,
            confidence=0.0,
            ranking=[RouteId.NO_TRADE],
        )
    else:
        route = RouteDecision(
            route_id=RouteId.INTRADAY,
            confidence=1.0,
            ranking=[RouteId.INTRADAY, RouteId.NO_TRADE],
        )
    return fc, route, proposal

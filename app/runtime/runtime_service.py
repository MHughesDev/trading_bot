from __future__ import annotations

import asyncio
import contextlib
import logging
from collections import defaultdict, deque
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any

import numpy as np

from app.config.settings import Settings
from app.contracts.audit import DecisionTrace
from app.contracts.common import ExecutionMode, SemanticRegime, SystemMode
from app.contracts.decisions import ExecutionReport
from app.contracts.events import BarEvent, OrderBookEvent, TickerEvent, TradeEvent
from app.runtime.mode_manager import ModeManager
from app.runtime.scheduler import AsyncScheduler, ScheduledJob
from app.runtime.state_manager import StateManager
from data_plane.bars.aggregator import TradeBarAggregator
from data_plane.features.feature_builder import FeatureBuilder
from data_plane.ingest.coinbase_ws import CoinbaseWebSocketIngest
from data_plane.memory.embedder import text_to_unit_embedding
from data_plane.memory.qdrant_memory import QdrantMemoryStore
from data_plane.memory.retriever import MemoryFeatureRetriever
from data_plane.storage.questdb_store import QuestDBStore
from data_plane.storage.redis_store import RedisStore
from decision_engine.engine import ActionGenerator, DecisionEngine
from execution.adapters.alpaca_paper_adapter import AlpacaPaperExecutionAdapter
from execution.adapters.coinbase_adapter import CoinbaseExecutionAdapter
from execution.router import ExecutionRouter
from models.forecast.tft_model import TFTForecastModel
from models.regime.gaussian_hmm import GaussianRegimeModel
from models.routing.selector import DeterministicRouteSelector
from observability.metrics import (
    DECISION_LATENCY_SECONDS,
    ORDER_FAILURE_TOTAL,
    ORDER_SUCCESS_TOTAL,
    PORTFOLIO_DRAWDOWN_PCT,
    PORTFOLIO_PNL_USD,
)
from risk_engine.engine import RiskEngine

logger = logging.getLogger(__name__)


def _feature_value(row: dict[str, Any], key: str, default: float = 0.0) -> float:
    val = row.get(key, default)
    if val is None:
        return default
    if isinstance(val, float) and np.isnan(val):
        return default
    return float(val)


@dataclass(slots=True)
class RuntimeStats:
    traces: deque[DecisionTrace] = field(default_factory=lambda: deque(maxlen=250))
    executions: deque[ExecutionReport] = field(default_factory=lambda: deque(maxlen=500))
    route_counts: dict[str, int] = field(default_factory=lambda: defaultdict(int))
    market_events_total: int = 0
    order_attempts_total: int = 0
    order_submissions_total: int = 0


class NautilusRuntimeService:
    """
    Main V1 runtime:
    Coinbase market data -> features/models -> route/action -> risk -> execution.
    """

    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.state_manager = StateManager(symbols=settings.market_data.symbols)
        self.mode_manager = ModeManager(self.state_manager)
        self._starting_equity = self.state_manager.get_state().portfolio.equity_usd
        self._peak_equity = self._starting_equity

        self._scheduler = AsyncScheduler()
        self._running = False
        self._started = False

        self._bars: dict[str, deque[BarEvent]] = {
            s: deque(maxlen=500) for s in settings.market_data.symbols
        }
        self._aggregator = TradeBarAggregator(interval_seconds=60)
        self._latest_market_ts: dict[str, datetime] = {}

        self._feature_builder = FeatureBuilder()
        self._forecast_model = TFTForecastModel(tuple(settings.models.forecast.horizons))
        self._regime_models = {
            s: GaussianRegimeModel(
                n_states=settings.models.regime.n_states,
                covariance_type=settings.models.regime.covariance_type,
                random_state=settings.models.regime.random_state,
            )
            for s in settings.market_data.symbols
        }
        self._decision_engine = DecisionEngine(
            route_selector=DeterministicRouteSelector(),
            action_generator=ActionGenerator(),
        )
        self._risk_engine = RiskEngine(settings.risk)

        coinbase_adapter = CoinbaseExecutionAdapter()
        alpaca_adapter = AlpacaPaperExecutionAdapter()
        self._paper_adapter = alpaca_adapter
        self._execution_router = ExecutionRouter(
            mode=settings.execution.mode,
            coinbase_adapter=coinbase_adapter,
            alpaca_paper_adapter=alpaca_adapter,
        )
        self.state_manager.update_execution_mode(settings.execution.mode)

        self._questdb = QuestDBStore(settings.storage.questdb)
        self._redis = RedisStore(settings.storage.redis)
        self._memory_store = QdrantMemoryStore(settings.storage.qdrant)
        self._memory_retriever = MemoryFeatureRetriever(self._memory_store, settings.models.memory)

        self._ws_ingest = CoinbaseWebSocketIngest(
            settings=settings.market_data,
            event_handler=self._on_market_event,
        )

        self.stats = RuntimeStats()

    async def start(self) -> None:
        if self._started:
            return
        self._started = True
        self._running = True

        self._memory_store.ensure_collection()
        await self._scheduler.start()
        self._scheduler.schedule(
            ScheduledJob(name="redis-heartbeat", interval_seconds=5.0, callback=self._heartbeat_job)
        )

        asyncio.create_task(self._ws_ingest.run(), name="market:coinbase_ws")
        self._scheduler.schedule(
            ScheduledJob(name="bar-flush", interval_seconds=5.0, callback=self._flush_bars_job)
        )
        logger.info("runtime_started", extra={"symbols": self.settings.market_data.symbols})

    async def stop(self) -> None:
        self._running = False
        await self._ws_ingest.stop()
        await self._scheduler.stop()
        await self._redis.close()
        logger.info("runtime_stopped")

    async def _heartbeat_job(self) -> None:
        await self._redis.heartbeat("nautilus-runtime")
        PORTFOLIO_PNL_USD.set(self.state_manager.get_state().portfolio.unrealized_pnl_usd)
        PORTFOLIO_DRAWDOWN_PCT.set(self.state_manager.get_state().portfolio.drawdown_pct)

    async def _flush_bars_job(self) -> None:
        # Ensure minute bars can close even during sparse trade periods.
        for bar in self._aggregator.flush():
            with contextlib.suppress(Exception):
                await self._on_bar_event(bar)

    async def _on_market_event(
        self, event: TickerEvent | TradeEvent | BarEvent | OrderBookEvent
    ) -> None:
        self.stats.market_events_total += 1
        symbol = getattr(event, "symbol", "")
        ts = getattr(event, "timestamp", datetime.now(UTC))
        self._latest_market_ts[symbol] = ts

        if isinstance(event, TickerEvent):
            self.state_manager.update_symbol_price(event.symbol, event.price)
            self._paper_adapter.update_last_price(event.symbol, event.price)
            await self._questdb.write_ticker(event)
            await self._redis.set_live_state(
                f"ticker:{event.symbol}", event.model_dump(mode="json")
            )
            if event.bid and event.ask and event.bid > 0:
                spread_bps = max((event.ask - event.bid) / event.bid * 10_000, 0.0)
                self.state_manager.update_symbol_spread_bps(event.symbol, spread_bps)
            return

        if isinstance(event, TradeEvent):
            self.state_manager.update_symbol_price(event.symbol, event.price)
            self._paper_adapter.update_last_price(event.symbol, event.price)
            await self._questdb.write_trade(event)
            closed_bar = self._aggregator.update(event)
            if closed_bar is not None:
                await self._on_bar_event(closed_bar)
            return

        if isinstance(event, BarEvent):
            await self._on_bar_event(event)
            return

        if isinstance(event, OrderBookEvent):
            await self._questdb.write_orderbook(event)
            if event.bids and event.asks and event.bids[0][0] > 0:
                spread_bps = max(
                    (event.asks[0][0] - event.bids[0][0]) / event.bids[0][0] * 10_000, 0.0
                )
                self.state_manager.update_symbol_spread_bps(event.symbol, spread_bps)
            await self._redis.set_live_state(
                f"orderbook:{event.symbol}", event.model_dump(mode="json")
            )

    async def _on_bar_event(self, bar: BarEvent) -> None:
        await self._questdb.write_bar(bar)
        await self._redis.set_live_state(f"bar:{bar.symbol}", bar.model_dump(mode="json"))
        self._bars[bar.symbol].append(bar)
        self._latest_market_ts[bar.symbol] = bar.timestamp
        await self._evaluate_symbol(bar.symbol)

    async def _evaluate_symbol(self, symbol: str) -> None:
        t0 = datetime.now(UTC)
        bars = list(self._bars[symbol])
        if len(bars) < 30:
            return

        features_df = self._feature_builder.compute(bars)
        if features_df.height == 0:
            return

        latest_row = features_df.tail(1).to_dicts()[0]
        features = {
            "ret_1": _feature_value(latest_row, "ret_1"),
            "ret_3": _feature_value(latest_row, "ret_3"),
            "ret_5": _feature_value(latest_row, "ret_5"),
            "ret_15": _feature_value(latest_row, "ret_15"),
            "vol_14": _feature_value(latest_row, "vol_14"),
            "ema_spread": _feature_value(latest_row, "ema_spread"),
            "vwap_distance": _feature_value(latest_row, "vwap_distance"),
            "rsi_14": _feature_value(latest_row, "rsi_14"),
        }

        arr = (
            features_df.select(["ret_1", "ret_3", "vol_14", "ema_spread"]).fill_null(0.0).to_numpy()
        )
        regime_model = self._regime_models[symbol]
        try:
            trained = getattr(regime_model, "_trained", False)
            if not trained and arr.shape[0] >= 40:
                regime_model.fit(arr)
        except Exception:
            logger.exception("regime_fit_error", extra={"symbol": symbol})
        regime = regime_model.infer(symbol=symbol, features=arr[-1:])
        self.state_manager.update_symbol_regime(symbol, regime.semantic_state)

        ret_series = features_df["ret_1"].fill_null(0.0).to_numpy()
        forecast = self._forecast_model.predict(
            symbol=symbol,
            recent_returns=np.asarray(ret_series, dtype=float),
            recent_volatility=features["vol_14"],
        )

        query_vec = text_to_unit_embedding(
            f"{symbol}|{features['ret_1']:.6f}|{features['vol_14']:.6f}"
        )
        memory_features = self._memory_retriever.get_features(symbol, query_vec)

        state = self.state_manager.get_state()
        sym_state = state.symbols.get(symbol)
        last_price = sym_state.last_price if sym_state and sym_state.last_price else bars[-1].close
        spread_bps = sym_state.spread_bps if sym_state else 0.0

        risk_pressure = min(
            state.portfolio.gross_exposure_usd
            / max(self.settings.risk.max_total_exposure_usd, 1e-6),
            1.0,
        )
        trace, action = self._decision_engine.run(
            symbol=symbol,
            last_price=last_price,
            features=features,
            memory_features=memory_features,
            forecast=forecast,
            regime=regime,
            spread_bps=spread_bps or 0.0,
            risk_pressure=risk_pressure,
        )
        self.stats.route_counts[trace.route_decision.route_id.value] += 1

        if action is None:
            self.stats.traces.append(trace)
            return

        order = self._decision_engine.action_to_order_intent(
            trace_id=trace.trace_id,
            route_id=trace.route_decision.route_id,
            action=action,
            last_price=last_price,
        )

        runtime_state = self.state_manager.get_state()
        risk_decision = self._risk_engine.evaluate(
            order=order,
            runtime_state=runtime_state,
            spread_bps=spread_bps,
            last_market_ts=self._latest_market_ts.get(symbol),
            mark_price=last_price,
        )
        trace.risk_decision = risk_decision
        self.stats.order_attempts_total += 1

        if not risk_decision.approved:
            self.stats.traces.append(trace)
            await self._redis.publish_event("decision_traces", trace.model_dump(mode="json"))
            return

        if (
            risk_decision.adjusted_quantity is not None
            and risk_decision.adjusted_quantity != order.quantity
        ):
            order = order.model_copy(update={"quantity": risk_decision.adjusted_quantity})
        trace.order_intent = order

        try:
            report = await self._execution_router.submit_order(order)
            self.stats.order_submissions_total += 1
            self.stats.executions.append(report)
            if report.filled_quantity > 0 and report.avg_fill_price:
                self.state_manager.apply_fill(
                    symbol=report.symbol,
                    side=report.side,
                    qty=report.filled_quantity,
                    fill_price=report.avg_fill_price,
                )
                self._peak_equity = self.state_manager.revalue_portfolio(
                    starting_equity=self._starting_equity,
                    peak_equity=self._peak_equity,
                )
            self.stats.traces.append(trace)
            ORDER_SUCCESS_TOTAL.labels(adapter=report.adapter, symbol=report.symbol).inc()
            await self._redis.publish_event("execution_reports", report.model_dump(mode="json"))
            await self._redis.publish_event("decision_traces", trace.model_dump(mode="json"))
        except Exception as exc:
            ORDER_FAILURE_TOTAL.labels(
                adapter=self._execution_router.active_adapter_name(),
                symbol=order.symbol,
                reason=exc.__class__.__name__,
            ).inc()
            logger.exception(
                "order_submit_failed", extra={"symbol": order.symbol, "error": str(exc)}
            )
            self.stats.traces.append(trace)
            await self._redis.publish_event("decision_traces", trace.model_dump(mode="json"))
        finally:
            dt = (datetime.now(UTC) - t0).total_seconds()
            DECISION_LATENCY_SECONDS.observe(dt)

    def set_execution_mode(self, mode: ExecutionMode) -> None:
        self._execution_router.set_mode(mode)
        self.state_manager.update_execution_mode(mode)

    def set_system_mode(self, mode: SystemMode) -> None:
        self.mode_manager.set_mode(mode)

    def get_snapshot(self) -> dict[str, Any]:
        state = self.state_manager.get_state()
        return {
            "system_mode": state.system_mode.value,
            "execution_mode": self._execution_router.mode.value,
            "portfolio": {
                "cash_usd": state.portfolio.cash_usd,
                "equity_usd": state.portfolio.equity_usd,
                "gross_exposure_usd": state.portfolio.gross_exposure_usd,
                "drawdown_pct": state.portfolio.drawdown_pct,
            },
            "symbols": {
                s: {
                    "last_price": sym.last_price,
                    "spread_bps": sym.spread_bps,
                    "regime": sym.regime.value if isinstance(sym.regime, SemanticRegime) else None,
                    "updated_at": sym.updated_at.isoformat(),
                }
                for s, sym in state.symbols.items()
            },
            "stats": {
                "market_events_total": self.stats.market_events_total,
                "order_attempts_total": self.stats.order_attempts_total,
                "order_submissions_total": self.stats.order_submissions_total,
                "route_counts": dict(self.stats.route_counts),
            },
        }

    def list_recent_routes(self, limit: int = 50) -> list[dict[str, Any]]:
        traces = list(self.stats.traces)[-limit:]
        return [
            {
                "trace_id": t.trace_id,
                "timestamp": t.timestamp.isoformat(),
                "symbol": t.symbol,
                "route": t.route_decision.route_id.value,
                "confidence": t.route_decision.confidence,
                "reasons": t.route_decision.reasons,
            }
            for t in traces
        ]

    def list_recent_traces(self, limit: int = 50) -> list[dict[str, Any]]:
        return [t.model_dump(mode="json") for t in list(self.stats.traces)[-limit:]]

    async def flatten_all(self) -> None:
        self.set_system_mode(SystemMode.FLATTEN_ALL)
        # In V1 scaffold, flatten intent is represented by mode transition and audit trace.
        logger.warning("flatten_all_triggered")

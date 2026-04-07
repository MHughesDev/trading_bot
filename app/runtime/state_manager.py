from __future__ import annotations

from datetime import UTC, datetime
from threading import RLock

from app.contracts.common import ExecutionMode, SemanticRegime, Side, SystemMode
from app.contracts.state import RuntimeState, SymbolState


class StateManager:
    """Thread-safe in-memory runtime state container."""

    def __init__(self, symbols: list[str]) -> None:
        self._lock = RLock()
        self._state = RuntimeState(symbols={s: SymbolState(symbol=s) for s in symbols})

    def get_state(self) -> RuntimeState:
        with self._lock:
            return self._state

    def update_symbol_price(self, symbol: str, price: float) -> None:
        with self._lock:
            if symbol not in self._state.symbols:
                self._state.symbols[symbol] = SymbolState(symbol=symbol)
            sym = self._state.symbols[symbol]
            sym.last_price = price
            sym.updated_at = datetime.now(UTC)
            self._state.updated_at = sym.updated_at

    def update_symbol_spread_bps(self, symbol: str, spread_bps: float) -> None:
        with self._lock:
            if symbol not in self._state.symbols:
                self._state.symbols[symbol] = SymbolState(symbol=symbol)
            sym = self._state.symbols[symbol]
            sym.spread_bps = spread_bps
            sym.updated_at = datetime.now(UTC)
            self._state.updated_at = sym.updated_at

    def update_mode(self, mode: SystemMode) -> None:
        with self._lock:
            self._state.system_mode = mode
            self._state.updated_at = datetime.now(UTC)

    def update_execution_mode(self, mode: ExecutionMode) -> None:
        with self._lock:
            self._state.execution_mode = mode
            self._state.updated_at = datetime.now(UTC)

    def update_symbol_regime(self, symbol: str, regime: SemanticRegime) -> None:
        with self._lock:
            if symbol not in self._state.symbols:
                self._state.symbols[symbol] = SymbolState(symbol=symbol)
            sym = self._state.symbols[symbol]
            sym.regime = regime
            sym.updated_at = datetime.now(UTC)
            self._state.updated_at = sym.updated_at

    def apply_fill(self, symbol: str, side: Side, qty: float, fill_price: float) -> None:
        with self._lock:
            if symbol not in self._state.symbols:
                self._state.symbols[symbol] = SymbolState(symbol=symbol)
            sym = self._state.symbols[symbol]
            if side == Side.BUY:
                sym.position_qty += qty
                self._state.portfolio.cash_usd -= qty * fill_price
            else:
                sym.position_qty -= qty
                self._state.portfolio.cash_usd += qty * fill_price

            sym.last_price = fill_price
            sym.exposure_usd = abs(sym.position_qty * fill_price)
            sym.updated_at = datetime.now(UTC)
            self._state.updated_at = sym.updated_at

    def revalue_portfolio(self, starting_equity: float, peak_equity: float) -> float:
        with self._lock:
            gross_exposure = 0.0
            market_value = 0.0
            for sym in self._state.symbols.values():
                if sym.last_price is None:
                    continue
                sym.exposure_usd = abs(sym.position_qty * sym.last_price)
                gross_exposure += sym.exposure_usd
                market_value += sym.position_qty * sym.last_price

            equity = self._state.portfolio.cash_usd + market_value
            new_peak = max(peak_equity, equity)
            drawdown = ((new_peak - equity) / new_peak) if new_peak > 0 else 0.0

            self._state.portfolio.gross_exposure_usd = gross_exposure
            self._state.portfolio.equity_usd = equity
            self._state.portfolio.unrealized_pnl_usd = equity - starting_equity
            self._state.portfolio.drawdown_pct = max(drawdown, 0.0)
            self._state.portfolio.updated_at = datetime.now(UTC)
            self._state.updated_at = self._state.portfolio.updated_at
            return new_peak

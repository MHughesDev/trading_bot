"""Reconstruct round-trip trades (entry/exit, realized PnL) from replay rows.

Consumes the per-bar row dicts produced by :func:`backtesting.replay.replay_decisions`
(single-asset) or :func:`backtesting.replay.replay_multi_asset_decisions` (multi-asset).
A *trade* is a realized close (or partial close) of a position: each time the signed
position is reduced toward or through zero we emit a :class:`TradeRecord` with realized
PnL computed against the running **average cost** of the open position.

This is analytics only — it never feeds the live decision/risk path. It exists so a
backtest can be judged on the metrics that actually matter for PnL (win rate, profit
factor, expectancy) rather than only the raw equity mark.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

__all__ = [
    "Fill",
    "TradeRecord",
    "TradeStats",
    "fills_from_rows",
    "build_trade_ledger",
    "summarize_trades",
]


def _to_float(value: Any) -> float | None:
    """Coerce str/Decimal/int/float to float; return None when not parseable."""
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


@dataclass(frozen=True)
class Fill:
    """A single simulated fill extracted from a replay row."""

    symbol: str
    side: str  # "buy" | "sell"
    quantity: float
    price: float
    fee: float
    timestamp: Any = None


@dataclass(frozen=True)
class TradeRecord:
    """A realized (fully or partially closed) round-trip position."""

    symbol: str
    direction: str  # "long" | "short"
    entry_timestamp: Any
    exit_timestamp: Any
    quantity: float
    entry_price: float
    exit_price: float
    gross_pnl: float
    fees: float
    net_pnl: float
    return_pct: float

    def to_dict(self) -> dict[str, Any]:
        return {
            "symbol": self.symbol,
            "direction": self.direction,
            "entry_timestamp": (
                self.entry_timestamp.isoformat()
                if hasattr(self.entry_timestamp, "isoformat")
                else self.entry_timestamp
            ),
            "exit_timestamp": (
                self.exit_timestamp.isoformat()
                if hasattr(self.exit_timestamp, "isoformat")
                else self.exit_timestamp
            ),
            "quantity": self.quantity,
            "entry_price": self.entry_price,
            "exit_price": self.exit_price,
            "gross_pnl": self.gross_pnl,
            "fees": self.fees,
            "net_pnl": self.net_pnl,
            "return_pct": self.return_pct,
        }


@dataclass
class TradeStats:
    """Aggregate per-trade statistics across a ledger."""

    num_trades: int = 0
    num_wins: int = 0
    num_losses: int = 0
    win_rate: float = 0.0
    gross_profit: float = 0.0
    gross_loss: float = 0.0
    net_pnl: float = 0.0
    profit_factor: float = 0.0
    avg_win: float = 0.0
    avg_loss: float = 0.0
    expectancy: float = 0.0
    largest_win: float = 0.0
    largest_loss: float = 0.0
    total_fees: float = 0.0

    def to_dict(self) -> dict[str, Any]:
        return dict(self.__dict__)


def _fill_from_payload(payload: dict[str, Any], symbol: str, ts: Any) -> Fill | None:
    """Build a Fill from a row's ``trade`` payload + ``fill_price``/``fee_paid`` columns."""
    trade = payload.get("trade")
    if not trade:
        return None
    side = str(trade.get("side", "")).lower()
    qty = _to_float(trade.get("quantity"))
    if qty is None or qty <= 0 or side not in ("buy", "sell"):
        return None
    # Prefer the simulated fill price; fall back to the trade's limit price if present.
    price = _to_float(payload.get("fill_price"))
    if price is None:
        price = _to_float(trade.get("limit_price"))
    if price is None:
        return None
    fee = _to_float(payload.get("fee_paid")) or 0.0
    return Fill(symbol=symbol, side=side, quantity=qty, price=price, fee=fee, timestamp=ts)


def fills_from_rows(rows: list[dict[str, Any]], *, symbol: str | None = None) -> list[Fill]:
    """Extract fills from single- or multi-asset replay rows.

    Single-asset rows carry ``trade``/``fill_price``/``fee_paid`` at the top level.
    Multi-asset rows carry a ``symbols`` dict of per-symbol payloads. When ``symbol``
    is given, only that instrument's fills are returned.
    """
    fills: list[Fill] = []
    for row in rows:
        ts = row.get("timestamp")
        sym_payloads = row.get("symbols")
        if isinstance(sym_payloads, dict):
            for sym, payload in sym_payloads.items():
                if symbol is not None and sym != symbol:
                    continue
                if not isinstance(payload, dict):
                    continue
                fill = _fill_from_payload(payload, str(sym), ts)
                if fill is not None:
                    fills.append(fill)
        else:
            sym = symbol or "UNKNOWN"
            fill = _fill_from_payload(row, sym, ts)
            if fill is not None:
                fills.append(fill)
    return fills


@dataclass
class _OpenPosition:
    """Running signed position with average cost + accumulated entry fees."""

    qty: float = 0.0  # signed: + long, - short
    avg_cost: float = 0.0
    entry_fees: float = 0.0
    entry_ts: Any = None

    def is_flat(self) -> bool:
        return abs(self.qty) < 1e-12


def build_trade_ledger(
    rows: list[dict[str, Any]], *, symbol: str | None = None
) -> list[TradeRecord]:
    """Reconstruct realized trades from replay rows using average-cost accounting.

    Each reduction of the open position emits a :class:`TradeRecord`. Position
    increases update the average cost; reversals close the old side and open the new.
    Entry fees are allocated proportionally to the closed quantity.
    """
    fills = fills_from_rows(rows, symbol=symbol)
    by_symbol: dict[str, list[Fill]] = {}
    for fill in fills:
        by_symbol.setdefault(fill.symbol, []).append(fill)

    records: list[TradeRecord] = []
    for sym, sym_fills in by_symbol.items():
        pos = _OpenPosition()
        for fill in sym_fills:
            signed = fill.quantity if fill.side == "buy" else -fill.quantity
            increasing = pos.is_flat() or (pos.qty > 0) == (signed > 0)
            if increasing:
                if pos.is_flat():
                    pos.entry_ts = fill.timestamp
                    pos.entry_fees = 0.0
                total_abs = abs(pos.qty) + abs(signed)
                pos.avg_cost = (
                    pos.avg_cost * abs(pos.qty) + fill.price * abs(signed)
                ) / total_abs
                pos.qty += signed
                pos.entry_fees += fill.fee
                continue

            # Reducing / closing / reversing.
            closing_qty = min(abs(signed), abs(pos.qty))
            direction = "long" if pos.qty > 0 else "short"
            if direction == "long":
                gross = (fill.price - pos.avg_cost) * closing_qty
            else:
                gross = (pos.avg_cost - fill.price) * closing_qty
            entry_fee_alloc = pos.entry_fees * (closing_qty / abs(pos.qty))
            exit_fee_alloc = fill.fee * (closing_qty / abs(signed))
            fees = entry_fee_alloc + exit_fee_alloc
            notional = pos.avg_cost * closing_qty
            records.append(
                TradeRecord(
                    symbol=sym,
                    direction=direction,
                    entry_timestamp=pos.entry_ts,
                    exit_timestamp=fill.timestamp,
                    quantity=closing_qty,
                    entry_price=pos.avg_cost,
                    exit_price=fill.price,
                    gross_pnl=gross,
                    fees=fees,
                    net_pnl=gross - fees,
                    return_pct=((gross - fees) / notional) if notional > 0 else 0.0,
                )
            )
            pos.entry_fees -= entry_fee_alloc
            pos.qty += signed
            if abs(signed) > closing_qty + 1e-12:
                # Reversal: leftover opens a fresh position at the fill price.
                remaining = abs(signed) - closing_qty
                pos.avg_cost = fill.price
                pos.qty = remaining if signed > 0 else -remaining
                pos.entry_ts = fill.timestamp
                pos.entry_fees = exit_fee_alloc  # remainder of this fill's fee
            elif pos.is_flat():
                pos.entry_ts = None
                pos.entry_fees = 0.0
    return records


def summarize_trades(records: list[TradeRecord]) -> TradeStats:
    """Aggregate win rate, profit factor, expectancy, etc. from realized trades."""
    stats = TradeStats()
    if not records:
        return stats
    wins: list[float] = []
    losses: list[float] = []
    for rec in records:
        stats.total_fees += rec.fees
        stats.net_pnl += rec.net_pnl
        if rec.net_pnl >= 0:
            wins.append(rec.net_pnl)
        else:
            losses.append(rec.net_pnl)
    stats.num_trades = len(records)
    stats.num_wins = len(wins)
    stats.num_losses = len(losses)
    stats.win_rate = stats.num_wins / stats.num_trades if stats.num_trades else 0.0
    stats.gross_profit = sum(wins)
    stats.gross_loss = sum(losses)  # negative
    stats.avg_win = (stats.gross_profit / len(wins)) if wins else 0.0
    stats.avg_loss = (stats.gross_loss / len(losses)) if losses else 0.0
    stats.expectancy = stats.net_pnl / stats.num_trades if stats.num_trades else 0.0
    stats.largest_win = max(wins) if wins else 0.0
    stats.largest_loss = min(losses) if losses else 0.0
    gross_loss_abs = abs(stats.gross_loss)
    if gross_loss_abs > 0:
        stats.profit_factor = stats.gross_profit / gross_loss_abs
    else:
        stats.profit_factor = float("inf") if stats.gross_profit > 0 else 0.0
    return stats

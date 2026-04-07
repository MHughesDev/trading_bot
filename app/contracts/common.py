from __future__ import annotations

from enum import Enum


class DataSource(str, Enum):
    COINBASE = "coinbase"


class ExecutionMode(str, Enum):
    PAPER = "paper"
    LIVE = "live"


class SystemMode(str, Enum):
    RUNNING = "RUNNING"
    PAUSE_NEW_ENTRIES = "PAUSE_NEW_ENTRIES"
    REDUCE_ONLY = "REDUCE_ONLY"
    FLATTEN_ALL = "FLATTEN_ALL"
    MAINTENANCE = "MAINTENANCE"


class RouteId(str, Enum):
    NO_TRADE = "NO_TRADE"
    SCALPING = "SCALPING"
    INTRADAY = "INTRADAY"
    SWING = "SWING"


class SemanticRegime(str, Enum):
    BULL = "bull"
    BEAR = "bear"
    VOLATILE = "volatile"
    SIDEWAYS = "sideways"


class Side(str, Enum):
    BUY = "buy"
    SELL = "sell"


class OrderType(str, Enum):
    MARKET = "market"
    LIMIT = "limit"
    STOP = "stop"
    STOP_LIMIT = "stop_limit"


class TimeInForce(str, Enum):
    GTC = "gtc"
    IOC = "ioc"
    FOK = "fok"
    DAY = "day"

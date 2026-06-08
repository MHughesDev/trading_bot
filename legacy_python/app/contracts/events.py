from datetime import datetime
from typing import Literal, Self

from pydantic import BaseModel, Field, model_validator

SourceLiteral = Literal["kraken", "coinbase"]


class BarEvent(BaseModel):
    """Canonical OHLCV bar (decision pipeline input).

    **FB-AP-014:** ``timestamp`` is the **bucket start** in UTC. ``interval_seconds`` is the bar
    width (default ``1`` for 1-second buckets). Durable storage keys on
    ``(symbol, timestamp, interval_seconds)`` together with OHLCV.
    """

    timestamp: datetime
    symbol: str
    open: float
    high: float
    low: float
    close: float
    volume: float
    interval_seconds: int = Field(
        default=1,
        ge=1,
        description="Bar width in seconds (bucket size; 1 = 1s bar)",
    )
    source: SourceLiteral = "kraken"
    schema_version: int = Field(default=1, ge=1)

    @model_validator(mode="after")
    def _validate_ohlc(self) -> Self:
        if self.high < self.low:
            raise ValueError("BarEvent high must be >= low")
        if not (self.low <= self.open <= self.high and self.low <= self.close <= self.high):
            raise ValueError("BarEvent open/close must lie within [low, high]")
        return self

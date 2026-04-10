from datetime import datetime
from typing import Literal, Self

from pydantic import BaseModel, Field, model_validator

SourceLiteral = Literal["coinbase"]


class BarEvent(BaseModel):
    """Canonical OHLCV bar (decision pipeline input)."""

    timestamp: datetime
    symbol: str
    open: float
    high: float
    low: float
    close: float
    volume: float
    source: SourceLiteral = "coinbase"
    schema_version: int = Field(default=1, ge=1)

    @model_validator(mode="after")
    def _validate_ohlc(self) -> Self:
        if self.high < self.low:
            raise ValueError("BarEvent high must be >= low")
        if not (self.low <= self.open <= self.high and self.low <= self.close <= self.high):
            raise ValueError("BarEvent open/close must lie within [low, high]")
        return self

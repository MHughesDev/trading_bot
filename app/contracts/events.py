from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field

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

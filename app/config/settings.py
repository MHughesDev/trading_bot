from __future__ import annotations

from pathlib import Path
from typing import Literal

import yaml
from pydantic import BaseModel, Field

from app.contracts.common import DataSource, ExecutionMode


class ExecutionSettings(BaseModel):
    mode: ExecutionMode = ExecutionMode.PAPER
    live_adapter: Literal["coinbase"] = "coinbase"
    paper_adapter: Literal["alpaca"] = "alpaca"


class MarketDataSettings(BaseModel):
    provider: DataSource = DataSource.COINBASE
    websocket_url: str
    rest_url: str
    symbols: list[str] = Field(default_factory=lambda: ["BTC-USD", "ETH-USD", "SOL-USD"])
    channels: list[str] = Field(
        default_factory=lambda: ["ticker", "market_trades", "level2", "candles"]
    )


class QuestDBSettings(BaseModel):
    host: str = "localhost"
    ilp_http_port: int = 9000
    enabled: bool = True


class RedisSettings(BaseModel):
    host: str = "localhost"
    port: int = 6379
    db: int = 0
    enabled: bool = True


class QdrantSettings(BaseModel):
    host: str = "localhost"
    port: int = 6333
    collection: str = "news_context_memory"
    enabled: bool = True


class MLflowSettings(BaseModel):
    tracking_uri: str = "http://localhost:5000"
    enabled: bool = True


class StorageSettings(BaseModel):
    questdb: QuestDBSettings = Field(default_factory=QuestDBSettings)
    redis: RedisSettings = Field(default_factory=RedisSettings)
    qdrant: QdrantSettings = Field(default_factory=QdrantSettings)
    mlflow: MLflowSettings = Field(default_factory=MLflowSettings)


class RiskSettings(BaseModel):
    max_total_exposure_usd: float = 100_000.0
    max_symbol_exposure_usd: float = 40_000.0
    max_drawdown_pct: float = 0.15
    max_spread_bps: float = 25.0
    stale_data_seconds: int = 10
    max_order_notional_usd: float = 5_000.0


class RegimeModelSettings(BaseModel):
    n_states: int = 4
    covariance_type: str = "full"
    random_state: int = 42


class ForecastModelSettings(BaseModel):
    horizons: list[int] = Field(default_factory=lambda: [1, 3, 5, 15])


class MemoryModelSettings(BaseModel):
    top_k: int = 12
    refresh_seconds: int = 60


class ModelSettings(BaseModel):
    regime: RegimeModelSettings = Field(default_factory=RegimeModelSettings)
    forecast: ForecastModelSettings = Field(default_factory=ForecastModelSettings)
    memory: MemoryModelSettings = Field(default_factory=MemoryModelSettings)


class ServiceSettings(BaseModel):
    log_level: str = "INFO"


class Settings(BaseModel):
    execution: ExecutionSettings
    market_data: MarketDataSettings
    storage: StorageSettings
    risk: RiskSettings
    models: ModelSettings
    service: ServiceSettings = Field(default_factory=ServiceSettings)


def load_settings(path: str | Path = "app/config/settings.yaml") -> Settings:
    with Path(path).open("r", encoding="utf-8") as f:
        raw = yaml.safe_load(f)
    return Settings.model_validate(raw)

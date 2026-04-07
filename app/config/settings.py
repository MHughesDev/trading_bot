"""Load YAML defaults + environment overrides (NM_ prefix)."""

from __future__ import annotations

from pathlib import Path
from typing import Any, Literal

import yaml
from pydantic import Field, SecretStr
from pydantic_settings import BaseSettings, SettingsConfigDict

_DEFAULT_YAML = Path(__file__).resolve().parent / "default.yaml"


class AppSettings(BaseSettings):
    """Application settings: default.yaml + NM_* environment variables."""

    model_config = SettingsConfigDict(
        env_prefix="NM_",
        env_file=".env",
        env_nested_delimiter="__",
        extra="ignore",
    )

    execution_mode: Literal["paper", "live"] = "paper"
    execution_live_adapter: str = "coinbase"
    execution_paper_adapter: str = "alpaca"

    market_data_provider: str = "coinbase"
    market_data_symbols: list[str] = Field(
        default_factory=lambda: ["BTC-USD", "ETH-USD", "SOL-USD"]
    )

    memory_qdrant_collection: str = "news_context_memory"
    memory_retrieval_interval_seconds: int = 60
    memory_top_k: int = 10

    risk_max_total_exposure_usd: float = 100_000
    risk_max_per_symbol_usd: float = 40_000
    risk_max_drawdown_pct: float = 0.15
    risk_max_spread_bps: float = 50
    risk_stale_data_seconds: float = 120

    features_return_windows: list[int] = Field(default_factory=lambda: [1, 3, 5, 15])
    features_volatility_windows: list[int] = Field(default_factory=lambda: [5, 15, 60])

    backtesting_slippage_bps: float = 5.0

    control_plane_host: str = "0.0.0.0"
    control_plane_port: int = 8000

    observability_log_level: str = "INFO"

    questdb_host: str = "localhost"
    questdb_port: int = 8812
    questdb_user: str = "admin"
    questdb_password: str = "quest"
    questdb_database: str = "qdb"

    redis_url: str = "redis://localhost:6379/0"

    qdrant_url: str = "http://localhost:6333"

    coinbase_api_key: SecretStr | None = None
    coinbase_api_secret: SecretStr | None = None

    alpaca_api_key: SecretStr | None = None
    alpaca_api_secret: SecretStr | None = None

    # HMAC for OrderIntent: only RiskEngine should create submittable intents
    risk_signing_secret: SecretStr | None = None
    allow_unsigned_execution: bool = False

    control_plane_api_key: SecretStr | None = None


def _yaml_to_kwargs(cfg: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    if "execution" in cfg:
        ex = cfg["execution"] or {}
        out["execution_mode"] = ex.get("mode", "paper")
        out["execution_live_adapter"] = ex.get("live_adapter", "coinbase")
        out["execution_paper_adapter"] = ex.get("paper_adapter", "alpaca")
    if "market_data" in cfg:
        md = cfg["market_data"] or {}
        out["market_data_provider"] = md.get("provider", "coinbase")
        if "symbols" in md:
            out["market_data_symbols"] = md["symbols"]
    if "memory" in cfg:
        mem = cfg["memory"] or {}
        out["memory_qdrant_collection"] = mem.get("qdrant_collection", "news_context_memory")
        out["memory_retrieval_interval_seconds"] = mem.get("retrieval_interval_seconds", 60)
        out["memory_top_k"] = mem.get("top_k", 10)
    if "risk" in cfg:
        r = cfg["risk"] or {}
        out["risk_max_total_exposure_usd"] = r.get("max_total_exposure_usd", 100_000)
        out["risk_max_per_symbol_usd"] = r.get("max_per_symbol_usd", 40_000)
        out["risk_max_drawdown_pct"] = r.get("max_drawdown_pct", 0.15)
        out["risk_max_spread_bps"] = r.get("max_spread_bps", 50)
        out["risk_stale_data_seconds"] = r.get("stale_data_seconds", 120)
    if "features" in cfg:
        fe = cfg["features"] or {}
        if "return_windows" in fe:
            out["features_return_windows"] = fe["return_windows"]
        if "volatility_windows" in fe:
            out["features_volatility_windows"] = fe["volatility_windows"]
    if "backtesting" in cfg:
        bt = cfg["backtesting"] or {}
        out["backtesting_slippage_bps"] = bt.get("slippage_bps", 5.0)
    if "control_plane" in cfg:
        cp = cfg["control_plane"] or {}
        out["control_plane_host"] = cp.get("host", "0.0.0.0")
        out["control_plane_port"] = cp.get("port", 8000)
    if "observability" in cfg:
        ob = cfg["observability"] or {}
        out["observability_log_level"] = ob.get("log_level", "INFO")
    return out


def load_settings(path: Path | None = None) -> AppSettings:
    """Load default.yaml then apply NM_* env vars (env wins)."""
    p = path or _DEFAULT_YAML
    kwargs: dict[str, Any] = {}
    if p.exists():
        with open(p, encoding="utf-8") as f:
            cfg = yaml.safe_load(f) or {}
        kwargs.update(_yaml_to_kwargs(cfg))
    return AppSettings(**kwargs)

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
    # Paper: Kraken mid is default (venue may not quote every product). Live: venue_only default.
    portfolio_mark_price_source_paper: Literal["kraken_mid", "venue_only"] = "kraken_mid"
    portfolio_mark_price_source_live: Literal["kraken_mid", "venue_only"] = "venue_only"
    position_reconcile_enabled: bool = False
    position_reconcile_interval_seconds: int = 60

    market_data_provider: str = "kraken"
    market_data_bar_interval_seconds: int = 1
    training_data_granularity_seconds: int = 1
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
    backtesting_fee_bps: float = 10.0
    backtesting_slippage_noise_bps: float = 0.0
    backtesting_rng_seed: int | None = None
    backtesting_initial_cash_usd: float = 100_000.0
    backtesting_enforce_solvency: bool = True
    backtesting_replay_available_cash: bool = False

    routing_spread_trade_max_bps: float = 30.0
    routing_forecast_strength_min: float = 0.001
    routing_score_scalping_forecast: float = 3.0
    routing_score_intraday_forecast: float = 2.0
    routing_score_swing_forecast: float = 2.0
    routing_spread_penalty_per_bp: float = 0.01

    redis_bar_ttl_seconds: int = 86_400

    sentiment_use_finbert: bool = False
    news_rss_feeds: list[str] = Field(default_factory=list)
    news_fetch_timeout_seconds: float = 15.0

    control_plane_host: str = "0.0.0.0"
    control_plane_port: int = 8000

    observability_log_level: str = "INFO"

    questdb_host: str = "localhost"
    questdb_port: int = 8812
    questdb_user: str = "admin"
    questdb_password: str = "quest"
    questdb_database: str = "qdb"
    questdb_persist_decision_traces: bool = False
    questdb_persist_microservice_events: bool = False
    questdb_batch_max_rows: int = 500
    questdb_flush_interval_seconds: float = 2.0

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

    # Serving lineage label for `ForecastPacket.source_checkpoint_id` (master pipeline spec §8)
    models_forecaster_checkpoint_id: str | None = None
    # Optional JSON path for `MultiHorizonConformal` state (forecaster calibration on hot path)
    models_forecaster_conformal_state_path: str | None = None
    # Optional NPZ: full NumPy `ForecasterWeightBundle` for deterministic forecaster forward (FB-SPEC-02)
    models_forecaster_weights_path: str | None = None
    # Optional PyTorch: `forecaster_torch.pt` from `train_torch_forecaster_distill` (FB-FR-P0); overrides NPZ/RNG quantiles when file exists
    models_forecaster_torch_path: str | None = None
    # Optional NPZ: `MultiBranchMLPPolicy` weights for PolicySystem actor (FB-SPEC-02)
    models_policy_mlp_path: str | None = None
    # Optional JSON: promoted serving paths (overrides env for keys present; FB-SPEC-06)
    models_active_set_path: str | None = None
    models_active_set_label: str | None = None
    models_active_set_manifest_version: int | None = None
    # PyTorch forecaster training / future torch inference: auto (CUDA if available) | cpu | cuda | cuda:N
    models_torch_device: str = "auto"
    # Optional JSON: operator "active model set" (FB-SPEC-06); merged into GET /status model_artifacts
    models_active_registry_path: str | None = None
    # Per-asset manifest (FB-AP-001/002): directory of `<symbol_slug>.json` manifests
    asset_model_registry_path: str | None = None
    # Optional single-file manifest (FB-AP-001): when set, used for binding checks (typical single-symbol deploy)
    asset_model_manifest_path: str | None = None

    microservices_runtime_bridge_enabled: bool = False
    # in_process: bridge runs decision→risk→execution handlers in-process (shadow).
    # external: bridge publishes through Redis; execution_gateway_service consumes risk.intent.accepted.
    microservices_execution_gateway_mode: Literal["in_process", "external"] = "in_process"


def _yaml_to_kwargs(cfg: dict[str, Any]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    if "execution" in cfg:
        ex = cfg["execution"] or {}
        out["execution_mode"] = ex.get("mode", "paper")
        out["execution_live_adapter"] = ex.get("live_adapter", "coinbase")
        out["execution_paper_adapter"] = ex.get("paper_adapter", "alpaca")
        if "portfolio_mark_price_source_paper" in ex:
            out["portfolio_mark_price_source_paper"] = str(ex["portfolio_mark_price_source_paper"])
        if "portfolio_mark_price_source_live" in ex:
            out["portfolio_mark_price_source_live"] = str(ex["portfolio_mark_price_source_live"])
        if "position_reconcile_enabled" in ex:
            out["position_reconcile_enabled"] = ex["position_reconcile_enabled"]
        if "position_reconcile_interval_seconds" in ex:
            out["position_reconcile_interval_seconds"] = int(ex["position_reconcile_interval_seconds"])
    if "market_data" in cfg:
        md = cfg["market_data"] or {}
        out["market_data_provider"] = md.get("provider", "kraken")
        if "bar_interval_seconds" in md:
            out["market_data_bar_interval_seconds"] = int(md["bar_interval_seconds"])
        if "training_granularity_seconds" in md:
            out["training_data_granularity_seconds"] = int(md["training_granularity_seconds"])
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
        if "fee_bps" in bt:
            out["backtesting_fee_bps"] = bt["fee_bps"]
        if "slippage_noise_bps" in bt:
            out["backtesting_slippage_noise_bps"] = bt["slippage_noise_bps"]
        if "rng_seed" in bt:
            v = bt["rng_seed"]
            out["backtesting_rng_seed"] = None if v is None else int(v)
        if "initial_cash_usd" in bt:
            out["backtesting_initial_cash_usd"] = float(bt["initial_cash_usd"])
        if "enforce_solvency" in bt:
            out["backtesting_enforce_solvency"] = bool(bt["enforce_solvency"])
        if "replay_available_cash" in bt:
            out["backtesting_replay_available_cash"] = bool(bt["replay_available_cash"])
    if "routing" in cfg:
        ro = cfg["routing"] or {}
        out["routing_spread_trade_max_bps"] = ro.get("spread_trade_max_bps", 30.0)
        out["routing_forecast_strength_min"] = ro.get("forecast_strength_min", 0.001)
        sw = ro.get("score_weights") or {}
        out["routing_score_scalping_forecast"] = sw.get("scalping_forecast", 3.0)
        out["routing_score_intraday_forecast"] = sw.get("intraday_forecast", 2.0)
        out["routing_score_swing_forecast"] = sw.get("swing_forecast", 2.0)
        out["routing_spread_penalty_per_bp"] = sw.get("spread_penalty_per_bp", 0.01)
    if "redis" in cfg:
        rd = cfg["redis"] or {}
        out["redis_bar_ttl_seconds"] = rd.get("bar_ttl_seconds", 86_400)
    if "sentiment" in cfg:
        se = cfg["sentiment"] or {}
        out["sentiment_use_finbert"] = se.get("use_finbert", False)
        if "rss_feeds" in se:
            out["news_rss_feeds"] = list(se["rss_feeds"] or [])
        if "fetch_timeout_seconds" in se:
            out["news_fetch_timeout_seconds"] = float(se["fetch_timeout_seconds"])
    if "control_plane" in cfg:
        cp = cfg["control_plane"] or {}
        out["control_plane_host"] = cp.get("host", "0.0.0.0")
        out["control_plane_port"] = cp.get("port", 8000)
    if "observability" in cfg:
        ob = cfg["observability"] or {}
        out["observability_log_level"] = ob.get("log_level", "INFO")
    if "questdb" in cfg:
        qd = cfg["questdb"] or {}
        if "persist_decision_traces" in qd:
            out["questdb_persist_decision_traces"] = qd["persist_decision_traces"]
        if "persist_microservice_events" in qd:
            out["questdb_persist_microservice_events"] = bool(qd["persist_microservice_events"])
        if "batch_max_rows" in qd:
            out["questdb_batch_max_rows"] = int(qd["batch_max_rows"])
        if "flush_interval_seconds" in qd:
            out["questdb_flush_interval_seconds"] = float(qd["flush_interval_seconds"])
    if "microservices" in cfg:
        ms = cfg["microservices"] or {}
        if "runtime_bridge_enabled" in ms:
            out["microservices_runtime_bridge_enabled"] = bool(ms["runtime_bridge_enabled"])
        if "execution_gateway_mode" in ms:
            out["microservices_execution_gateway_mode"] = str(ms["execution_gateway_mode"])
    if "models" in cfg:
        mo = cfg["models"] or {}
        if "forecaster_checkpoint_id" in mo:
            v = mo["forecaster_checkpoint_id"]
            out["models_forecaster_checkpoint_id"] = None if v is None else str(v)
        if "forecaster_conformal_state_path" in mo:
            v = mo["forecaster_conformal_state_path"]
            out["models_forecaster_conformal_state_path"] = None if v is None else str(v)
        if "forecaster_weights_path" in mo:
            v = mo["forecaster_weights_path"]
            out["models_forecaster_weights_path"] = None if v is None else str(v)
        if "forecaster_torch_path" in mo:
            v = mo["forecaster_torch_path"]
            out["models_forecaster_torch_path"] = None if v is None else str(v)
        if "policy_mlp_path" in mo:
            v = mo["policy_mlp_path"]
            out["models_policy_mlp_path"] = None if v is None else str(v)
        if "active_set_path" in mo:
            v = mo["active_set_path"]
            out["models_active_set_path"] = None if v is None else str(v)
        if "active_set_label" in mo:
            v = mo["active_set_label"]
            out["models_active_set_label"] = None if v is None else str(v)
        if "active_set_manifest_version" in mo:
            v = mo["active_set_manifest_version"]
            if v is None:
                out["models_active_set_manifest_version"] = None
            else:
                out["models_active_set_manifest_version"] = int(v)
        if "torch_device" in mo:
            out["models_torch_device"] = str(mo["torch_device"])
        if "active_registry_path" in mo:
            v = mo["active_registry_path"]
            out["models_active_registry_path"] = None if v is None else str(v)
        if "asset_model_registry_path" in mo:
            v = mo["asset_model_registry_path"]
            out["asset_model_registry_path"] = None if v is None else str(v)
        if "asset_model_manifest_path" in mo:
            v = mo["asset_model_manifest_path"]
            out["asset_model_manifest_path"] = None if v is None else str(v)
    return out


def load_settings(path: Path | None = None) -> AppSettings:
    """Load default.yaml then apply NM_* env vars (env wins)."""
    p = path or _DEFAULT_YAML
    kwargs: dict[str, Any] = {}
    if p.exists():
        with open(p, encoding="utf-8") as f:
            cfg = yaml.safe_load(f) or {}
        kwargs.update(_yaml_to_kwargs(cfg))
    base = AppSettings(**kwargs)
    # Optional JSON manifest overrides serving paths (FB-SPEC-06); applied after env.
    from models.registry.active_set import apply_active_model_set

    return apply_active_model_set(base)

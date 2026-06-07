"""The concrete tool set an AI agent is equipped with to operate the platform.

Read tools give the agent the context it needs to decide (status, assets, bars, positions,
PnL, the latest decision record). Act tools let it trade and manage assets — every act
routes through the control-plane endpoints, so the RiskEngine gates and risk-signing apply
identically to agent and human. The agent is *well-equipped but not omnipotent*: it can
buy, sell, flatten, and manage lifecycle, but cannot bypass risk.
"""

from __future__ import annotations

from typing import Any

from mcp_server.backend import PlatformBackend
from mcp_server.registry import ToolSpec

_SYMBOL = {"type": "string", "description": "Canonical symbol, e.g. BTC-USD"}


def _require(args: dict[str, Any], key: str) -> Any:
    if key not in args or args[key] in (None, ""):
        raise ValueError(f"missing required argument: {key}")
    return args[key]


_PAGINATION = {
    "limit": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 200},
    "offset": {"type": "integer", "minimum": 0, "default": 0},
    "query": {"type": "string", "description": "Case-insensitive filter substring"},
}


def build_default_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Return the full read/act tool specs bound to ``backend`` — the agent's complete
    operating surface over the control plane (status, universe, governance, params,
    system mode/power, asset manifests/lifecycle/execution, charts, trading)."""

    return [
        *_core_tools(backend),
        *_universe_tools(backend),
        *_system_tools(backend),
        *_asset_admin_tools(backend),
        *_chart_tools(backend),
        *_governance_tools(backend),
    ]


def _core_tools(backend: PlatformBackend) -> list[ToolSpec]:
    return [
        ToolSpec(
            name="get_system_status",
            description=(
                "Overall platform status: execution mode, system power/mode, per-asset "
                "lifecycle states, and health. Call this first to orient."
            ),
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.system_status(),
        ),
        ToolSpec(
            name="list_watched_assets",
            description="List assets that have been initialized (model manifests) on the platform.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.list_assets(),
        ),
        ToolSpec(
            name="get_recent_bars",
            description=(
                "Recent OHLCV minute bars for a symbol — the price context for a decision. "
                "Returns up to `limit` bars at `interval_seconds` (default 60s)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200},
                    "interval_seconds": {"type": "integer", "description": "Bar width (default 60)"},
                },
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_bars(
                _require(a, "symbol"),
                limit=int(a.get("limit", 200)),
                interval_seconds=a.get("interval_seconds"),
            ),
        ),
        ToolSpec(
            name="get_positions",
            description="Current open positions from the configured execution venue.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_positions(),
        ),
        ToolSpec(
            name="get_pnl",
            description="Realized + unrealized P&L summary over a rolling window.",
            input_schema={
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["hour", "day", "month", "year", "all"],
                        "default": "day",
                    }
                },
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_pnl(window=str(a.get("window", "day"))),
        ),
        ToolSpec(
            name="get_latest_decision",
            description=(
                "The most recent canonical DecisionRecord from the live process (regime, "
                "forecast, route, risk, reason codes) — what the platform last concluded."
            ),
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_latest_decision(),
        ),
        ToolSpec(
            name="place_order",
            description=(
                "Place a buy or sell order. quantity is absolute base units. order_type is "
                "'market', 'limit' (needs limit_price), 'stop' (needs stop_price), or 'stop_limit' "
                "(needs both). time_in_force is gtc/ioc/fok/gtd. Pass mid_price to enable the "
                "available-cash check. The order passes RiskEngine gates and is risk-signed; it "
                "may be blocked (see `blocked` reason codes in the result)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "side": {"type": "string", "enum": ["buy", "sell"]},
                    "quantity": {"type": "string", "description": "Absolute quantity, e.g. '0.01'"},
                    "order_type": {
                        "type": "string",
                        "enum": ["market", "limit", "stop", "stop_limit"],
                        "default": "market",
                    },
                    "limit_price": {
                        "type": "string",
                        "description": "Required for limit and stop_limit orders",
                    },
                    "stop_price": {
                        "type": "string",
                        "description": "Required for stop and stop_limit orders",
                    },
                    "time_in_force": {
                        "type": "string",
                        "enum": ["gtc", "ioc", "fok", "gtd"],
                        "default": "gtc",
                    },
                    "mid_price": {"type": "number", "description": "Reference mark for cash gate"},
                },
                "required": ["symbol", "side", "quantity"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.place_order(
                symbol=_require(a, "symbol"),
                side=_require(a, "side"),
                quantity=str(_require(a, "quantity")),
                order_type=str(a.get("order_type", "market")),
                limit_price=a.get("limit_price"),
                stop_price=a.get("stop_price"),
                time_in_force=str(a.get("time_in_force", "gtc")),
                mid_price=a.get("mid_price"),
            ),
            mutating=True,
        ),
        ToolSpec(
            name="flatten_position",
            description="Market-close the entire open position for a symbol.",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.flatten(_require(a, "symbol")),
            mutating=True,
        ),
        ToolSpec(
            name="set_asset_lifecycle",
            description=(
                "Manage an asset's lifecycle: 'initialize' (set up models), 'start' (begin "
                "watching/trading), or 'stop' (stop and flatten)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "action": {"type": "string", "enum": ["initialize", "start", "stop"]},
                },
                "required": ["symbol", "action"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_lifecycle(_require(a, "symbol"), _require(a, "action")),
            mutating=True,
        ),
        ToolSpec(
            name="set_execution_mode",
            description="Override per-asset execution mode: 'paper', 'live', or 'default'.",
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "mode": {"type": "string", "enum": ["paper", "live", "default"]},
                },
                "required": ["symbol", "mode"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_execution_mode(_require(a, "symbol"), _require(a, "mode")),
            mutating=True,
        ),
    ]


def _universe_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Tradable-symbol universes (Alpaca/Coinbase snapshots, cross-venue search) + on-demand sync."""

    def _pagination_schema(extra_description: str = "") -> dict[str, Any]:
        props = dict(_PAGINATION)
        if extra_description:
            props = {**props, "query": {**props["query"], "description": extra_description}}
        return {"type": "object", "properties": props, "additionalProperties": False}

    def _pag_args(a: dict[str, Any]) -> dict[str, Any]:
        return {"limit": int(a.get("limit", 200)), "offset": int(a.get("offset", 0)), "query": a.get("query")}

    return [
        ToolSpec(
            name="list_alpaca_universe",
            description="Paginated Alpaca tradable-crypto snapshot (metadata only, no OHLC).",
            input_schema=_pagination_schema("Filter by symbol or name"),
            handler=lambda a: backend.list_alpaca_universe(**_pag_args(a)),
        ),
        ToolSpec(
            name="sync_alpaca_universe",
            description="On-demand refresh of the Alpaca tradable-crypto universe snapshot from the venue API.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.sync_alpaca_universe(),
            mutating=True,
        ),
        ToolSpec(
            name="list_coinbase_universe",
            description="Paginated Coinbase SPOT product snapshot (metadata only, no OHLC).",
            input_schema=_pagination_schema("Filter by product_id or base name"),
            handler=lambda a: backend.list_coinbase_universe(**_pag_args(a)),
        ),
        ToolSpec(
            name="sync_coinbase_universe",
            description="On-demand refresh of the Coinbase SPOT product universe snapshot from the venue API.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.sync_coinbase_universe(),
            mutating=True,
        ),
        ToolSpec(
            name="list_platform_supported_universe",
            description="Cross-venue platform-supported symbols — eligibility/search only, not OHLC.",
            input_schema=_pagination_schema("Filter symbol/name/base"),
            handler=lambda a: backend.list_platform_supported_universe(**_pag_args(a)),
        ),
        ToolSpec(
            name="search_universe",
            description="Paginated symbol search across the platform-supported set with venue metadata.",
            input_schema=_pagination_schema("Filter symbol/name/base"),
            handler=lambda a: backend.search_universe(**_pag_args(a)),
        ),
    ]


def _system_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Platform-wide controls: scheduler, mode/power, params, model labels, P&L series, health."""

    return [
        ToolSpec(
            name="get_scheduler_status",
            description="Nightly in-process scheduler status: last/next run, last error, last training report.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_scheduler_status(),
        ),
        ToolSpec(
            name="get_microservices_health",
            description="Best-effort health probes for optional scaffold microservice processes.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_microservices_health(),
        ),
        ToolSpec(
            name="get_pnl_series",
            description="Cumulative realized P&L time series from the local ledger, bucketed over a rolling window.",
            input_schema={
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["hour", "day", "month", "year", "all"],
                        "default": "day",
                    },
                    "bucket_seconds": {"type": "integer", "minimum": 60, "maximum": 86400, "default": 3600},
                    "mode": {"type": "string", "enum": ["paper", "live"], "description": "Dashboard view filter only"},
                },
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_pnl_series(
                window=str(a.get("window", "day")),
                bucket_seconds=int(a.get("bucket_seconds", 3600)),
                mode=a.get("mode"),
            ),
        ),
        ToolSpec(
            name="get_routes",
            description="The platform's named decision routes (NO_TRADE/SCALPING/INTRADAY/SWING).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_routes(),
        ),
        ToolSpec(
            name="get_params",
            description="Current runtime parameter overrides.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_params(),
        ),
        ToolSpec(
            name="set_params",
            description="Replace the runtime parameter overrides with the given object (operator action).",
            input_schema={
                "type": "object",
                "properties": {"params": {"type": "object", "description": "New parameter overrides"}},
                "required": ["params"],
                "additionalProperties": True,
            },
            handler=lambda a: backend.set_params(a.get("params", a)),
            mutating=True,
        ),
        ToolSpec(
            name="get_system_mode",
            description="Current platform SystemMode (e.g. RUNNING, FLATTEN_ALL).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_system_mode(),
        ),
        ToolSpec(
            name="set_system_mode",
            description="Set the platform SystemMode (operator action; affects all assets).",
            input_schema={
                "type": "object",
                "properties": {"mode": {"type": "string", "description": "e.g. RUNNING, PAUSED, FLATTEN_ALL"}},
                "required": ["mode"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_system_mode(_require(a, "mode")),
            mutating=True,
        ),
        ToolSpec(
            name="system_flatten",
            description=(
                "Emergency: switch the whole platform into FLATTEN_ALL mode so the execution layer "
                "closes every open position. Affects all assets — use with care."
            ),
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.system_flatten(),
            mutating=True,
        ),
        ToolSpec(
            name="get_system_power",
            description="Legacy global power switch state (only meaningful if the legacy switch is enabled).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_system_power(),
        ),
        ToolSpec(
            name="set_system_power",
            description=(
                "Legacy global power switch ('on'/'off'). Disabled (HTTP 410) unless the platform "
                "has the legacy switch enabled — prefer per-asset lifecycle controls."
            ),
            input_schema={
                "type": "object",
                "properties": {"power": {"type": "string", "enum": ["on", "off"]}},
                "required": ["power"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_system_power(_require(a, "power")),
            mutating=True,
        ),
        ToolSpec(
            name="get_execution_profile",
            description="Legacy app-wide paper/live execution profile (only available if the legacy API is enabled).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_execution_profile(),
        ),
        ToolSpec(
            name="set_execution_profile",
            description=(
                "Legacy app-wide paper/live intent. Disabled (HTTP 410) unless the platform has the "
                "legacy execution-profile API enabled — prefer per-asset `set_execution_mode`."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "execution_mode": {"type": "string", "enum": ["paper", "live"]},
                    "apply_to_config_files": {"type": "boolean", "default": True},
                },
                "required": ["execution_mode"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_execution_profile(
                execution_mode=_require(a, "execution_mode"),
                apply_to_config_files=bool(a.get("apply_to_config_files", True)),
            ),
            mutating=True,
        ),
        ToolSpec(
            name="list_models",
            description="Named model components the platform tracks version labels for.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.list_models(),
        ),
        ToolSpec(
            name="set_model_version",
            description="Record a model component's version label (exposed to Prometheus as model_version_info).",
            input_schema={
                "type": "object",
                "properties": {
                    "component": {"type": "string", "description": "e.g. forecaster, policy, risk_engine"},
                    "version": {"type": "string"},
                },
                "required": ["component", "version"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_model_version(
                component=_require(a, "component"), version=_require(a, "version")
            ),
            mutating=True,
        ),
    ]


def _asset_admin_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Per-asset administration: model manifests, lifecycle/execution-mode introspection, init jobs."""

    return [
        ToolSpec(
            name="get_asset_model_manifest",
            description="Load the persisted per-asset model manifest for a symbol (404 if not initialized).",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_asset_model_manifest(_require(a, "symbol")),
        ),
        ToolSpec(
            name="put_asset_model_manifest",
            description=(
                "Create or replace a symbol's model manifest. `manifest` must be a full "
                "AssetModelManifest object whose `canonical_symbol` matches `symbol`."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "manifest": {"type": "object", "description": "Full AssetModelManifest JSON"},
                },
                "required": ["symbol", "manifest"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.put_asset_model_manifest(_require(a, "symbol"), _require(a, "manifest")),
            mutating=True,
        ),
        ToolSpec(
            name="delete_asset_model_manifest",
            description="Remove a symbol's model manifest and lifecycle state (irreversible — re-init required).",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.delete_asset_model_manifest(_require(a, "symbol")),
            mutating=True,
        ),
        ToolSpec(
            name="get_asset_lifecycle",
            description="Per-asset lifecycle state (uninitialized / initialized_not_active / active).",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_asset_lifecycle(_require(a, "symbol")),
        ),
        ToolSpec(
            name="get_asset_execution_mode",
            description="Per-symbol paper/live routing override (falls back to the platform default when unset).",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_asset_execution_mode(_require(a, "symbol")),
        ),
        ToolSpec(
            name="get_asset_init_job",
            description="Poll an asset-initialization job's status and per-step detail by job_id.",
            input_schema={
                "type": "object",
                "properties": {"job_id": {"type": "string"}},
                "required": ["job_id"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_asset_init_job(_require(a, "job_id")),
        ),
    ]


def _chart_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Chart-adjacent context: latest bar and historical buy/sell trade markers."""

    return [
        ToolSpec(
            name="get_latest_bar",
            description="The most recent stored canonical OHLCV bar for a symbol (last-price context).",
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "interval_seconds": {"type": "integer", "description": "Bar width (default platform setting)"},
                },
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_latest_bar(_require(a, "symbol"), interval_seconds=a.get("interval_seconds")),
        ),
        ToolSpec(
            name="get_trade_markers",
            description="Historical buy/sell trade markers for a symbol over an explicit UTC ISO-8601 time range.",
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "start": {"type": "string", "description": "Range start, UTC ISO-8601"},
                    "end": {"type": "string", "description": "Range end, UTC ISO-8601"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 2000},
                },
                "required": ["symbol", "start", "end"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_trade_markers(
                _require(a, "symbol"),
                start=_require(a, "start"),
                end=_require(a, "end"),
                limit=int(a.get("limit", 2000)),
            ),
        ),
    ]


def _governance_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """APEX governance surface: release evidence/ledger, experiments, shadow comparisons, rollout gates."""

    return [
        ToolSpec(
            name="get_release_evidence",
            description="APEX release-evidence bundle for the running process (config/logic/model fingerprints).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_release_evidence(),
        ),
        ToolSpec(
            name="diff_release_evidence",
            description=(
                "Structured diff between a baseline default.yaml document (full text) and the running "
                "merged canonical config. Optionally appends the report to the immutable audit log."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "baseline_yaml": {"type": "string", "description": "Full baseline default.yaml document text"},
                    "append_audit": {"type": "boolean", "default": False},
                },
                "required": ["baseline_yaml"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.diff_release_evidence(
                baseline_yaml=_require(a, "baseline_yaml"), append_audit=bool(a.get("append_audit", False))
            ),
            mutating=True,
        ),
        ToolSpec(
            name="get_config_diff_audit",
            description="Tail of the immutable config-diff audit log (most recent N entries).",
            input_schema={
                "type": "object",
                "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50}},
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_config_diff_audit(limit=int(a.get("limit", 50))),
        ),
        ToolSpec(
            name="get_governance_monitoring",
            description="Pointers to APEX canonical dashboards, Prometheus rules, and governance metric names.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_governance_monitoring(),
        ),
        ToolSpec(
            name="get_probation_status",
            description="Post-release live-probation evaluation snapshot (abort recommendation, phase, metrics).",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_probation_status(),
        ),
        ToolSpec(
            name="get_shadow_comparison",
            description="Last persisted shadow-vs-baseline replay comparison report and its policy.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_shadow_comparison(),
        ),
        ToolSpec(
            name="run_shadow_comparison",
            description=(
                "Run a synthetic paired baseline-vs-candidate replay for shadow-divergence metrics, "
                "persist the structured report, and return it."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": {**_SYMBOL, "default": "BTC-USD"},
                    "bars": {"type": "integer", "minimum": 50, "maximum": 5000, "default": 220},
                    "baseline_logic_version": {"type": "string", "default": "1.0.0"},
                    "candidate_logic_version": {"type": "string", "default": "1.0.0"},
                    "baseline_replay_run_id": {"type": "string", "default": "shadow-baseline"},
                    "candidate_replay_run_id": {"type": "string", "default": "shadow-candidate"},
                },
                "additionalProperties": False,
            },
            handler=lambda a: backend.run_shadow_comparison(
                symbol=a.get("symbol"),
                bars=a.get("bars"),
                baseline_logic_version=a.get("baseline_logic_version"),
                candidate_logic_version=a.get("candidate_logic_version"),
                baseline_replay_run_id=a.get("baseline_replay_run_id"),
                candidate_replay_run_id=a.get("candidate_replay_run_id"),
            ),
            mutating=True,
        ),
        ToolSpec(
            name="get_rollback_playbook",
            description="Rollback-playbook requirements per release stage and pointers to docs/scripts.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_rollback_playbook(),
        ),
        ToolSpec(
            name="list_release_objects",
            description="The APEX release ledger: config/logic/model/feature/combined release candidate records.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.list_release_objects(),
        ),
        ToolSpec(
            name="get_release_object",
            description="A single release candidate record by release_id.",
            input_schema={
                "type": "object",
                "properties": {"release_id": {"type": "string"}},
                "required": ["release_id"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_release_object(_require(a, "release_id")),
        ),
        ToolSpec(
            name="create_release_object",
            description=(
                "Create or replace a release candidate in the ledger by release_id. "
                "`candidate` must be a full ReleaseCandidate JSON object."
            ),
            input_schema={
                "type": "object",
                "properties": {"candidate": {"type": "object", "description": "Full ReleaseCandidate JSON"}},
                "required": ["candidate"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.create_release_object(_require(a, "candidate")),
            mutating=True,
        ),
        ToolSpec(
            name="evaluate_release_gates",
            description="Evaluate APEX promotion gates for a candidate JSON without persisting it.",
            input_schema={
                "type": "object",
                "properties": {
                    "candidate": {"type": "object", "description": "Full ReleaseCandidate JSON"},
                    "target_environment": {
                        "type": "string",
                        "enum": ["research", "simulation", "shadow", "live"],
                        "default": "live",
                    },
                    "experiment_registry_path": {"type": "string"},
                },
                "required": ["candidate"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.evaluate_release_gates(
                candidate=_require(a, "candidate"),
                target_environment=str(a.get("target_environment", "live")),
                experiment_registry_path=a.get("experiment_registry_path"),
            ),
            mutating=True,
        ),
        ToolSpec(
            name="list_experiments",
            description="Filterable list of the APEX experiment registry (domain, status, change_type, tags, ...).",
            input_schema={
                "type": "object",
                "properties": {
                    "domain": {"type": "string"},
                    "status": {"type": "string"},
                    "change_type": {"type": "string"},
                    "tag": {"type": "string"},
                    "linked_release": {"type": "string"},
                    "notes_substring": {"type": "string"},
                },
                "additionalProperties": False,
            },
            handler=lambda a: backend.list_experiments(
                domain=a.get("domain"),
                status=a.get("status"),
                change_type=a.get("change_type"),
                tag=a.get("tag"),
                linked_release=a.get("linked_release"),
                notes_substring=a.get("notes_substring"),
            ),
        ),
        ToolSpec(
            name="get_experiment",
            description="A single experiment record by experiment_id.",
            input_schema={
                "type": "object",
                "properties": {"experiment_id": {"type": "string"}},
                "required": ["experiment_id"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_experiment(_require(a, "experiment_id")),
        ),
        ToolSpec(
            name="create_experiment",
            description=(
                "Create or replace an experiment record by experiment_id. "
                "`experiment` must match the ExperimentRecord schema."
            ),
            input_schema={
                "type": "object",
                "properties": {"experiment": {"type": "object", "description": "Full ExperimentRecord JSON"}},
                "required": ["experiment"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.create_experiment(_require(a, "experiment")),
            mutating=True,
        ),
        ToolSpec(
            name="delete_experiment",
            description="Delete an experiment record by experiment_id (irreversible).",
            input_schema={
                "type": "object",
                "properties": {"experiment_id": {"type": "string"}},
                "required": ["experiment_id"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.delete_experiment(_require(a, "experiment_id")),
            mutating=True,
        ),
    ]

"""Backend the MCP tools act on: the trading platform's control-plane API.

``PlatformBackend`` is the protocol the tools depend on (so tests can substitute a fake).
``HttpPlatformBackend`` implements it against the FastAPI control plane over HTTP, which
keeps the AI agent fully decoupled from the trading process — it is just another client of
the same endpoints humans use. Mutating calls send the operator ``X-API-Key`` (the MCP
server is automation, not a browser session).
"""

from __future__ import annotations

import datetime as _dt
from typing import Any, Protocol, runtime_checkable

import httpx


@runtime_checkable
class PlatformBackend(Protocol):
    """Capabilities the MCP tools require from the trading platform."""

    def system_status(self) -> dict[str, Any]: ...
    def list_assets(self) -> dict[str, Any]: ...
    def get_bars(
        self, symbol: str, *, limit: int = 200, interval_seconds: int | None = None
    ) -> dict[str, Any]: ...
    def get_positions(self) -> dict[str, Any]: ...
    def get_pnl(self, *, window: str = "day") -> dict[str, Any]: ...
    def get_latest_decision(self) -> dict[str, Any]: ...
    def place_order(
        self,
        *,
        symbol: str,
        side: str,
        quantity: str,
        order_type: str = "market",
        limit_price: str | None = None,
        stop_price: str | None = None,
        time_in_force: str = "gtc",
        mid_price: float | None = None,
    ) -> dict[str, Any]: ...
    def flatten(self, symbol: str) -> dict[str, Any]: ...
    def set_lifecycle(self, symbol: str, action: str) -> dict[str, Any]: ...
    def set_execution_mode(self, symbol: str, mode: str) -> dict[str, Any]: ...

    # --- extended reads: scheduler, universe, system, governance, assets, charts ---
    def get_scheduler_status(self) -> dict[str, Any]: ...
    def get_system_power(self) -> dict[str, Any]: ...
    def get_execution_profile(self) -> dict[str, Any]: ...
    def get_pnl_series(
        self, *, window: str = "day", bucket_seconds: int = 3600, mode: str | None = None
    ) -> dict[str, Any]: ...
    def get_microservices_health(self) -> dict[str, Any]: ...
    def list_alpaca_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]: ...
    def list_coinbase_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]: ...
    def list_platform_supported_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]: ...
    def search_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]: ...
    def get_routes(self) -> dict[str, Any]: ...
    def get_params(self) -> dict[str, Any]: ...
    def get_system_mode(self) -> dict[str, Any]: ...
    def list_models(self) -> dict[str, Any]: ...
    def get_asset_model_manifest(self, symbol: str) -> dict[str, Any]: ...
    def get_asset_lifecycle(self, symbol: str) -> dict[str, Any]: ...
    def get_asset_execution_mode(self, symbol: str) -> dict[str, Any]: ...
    def get_asset_init_job(self, job_id: str) -> dict[str, Any]: ...
    def get_latest_bar(self, symbol: str, *, interval_seconds: int | None = None) -> dict[str, Any]: ...
    def get_trade_markers(
        self, symbol: str, *, start: str, end: str, limit: int = 2000
    ) -> dict[str, Any]: ...
    def get_release_evidence(self) -> dict[str, Any]: ...
    def get_governance_monitoring(self) -> dict[str, Any]: ...
    def get_probation_status(self) -> dict[str, Any]: ...
    def get_shadow_comparison(self) -> dict[str, Any]: ...
    def get_config_diff_audit(self, *, limit: int = 50) -> dict[str, Any]: ...
    def list_release_objects(self) -> dict[str, Any]: ...
    def get_release_object(self, release_id: str) -> dict[str, Any]: ...
    def get_rollback_playbook(self) -> dict[str, Any]: ...
    def list_experiments(self, **filters: Any) -> dict[str, Any]: ...
    def get_experiment(self, experiment_id: str) -> dict[str, Any]: ...

    # --- extended actions (mutating) ---
    def sync_alpaca_universe(self) -> dict[str, Any]: ...
    def sync_coinbase_universe(self) -> dict[str, Any]: ...
    def run_shadow_comparison(self, **kwargs: Any) -> dict[str, Any]: ...
    def diff_release_evidence(self, *, baseline_yaml: str, append_audit: bool = False) -> dict[str, Any]: ...
    def create_release_object(self, candidate: dict[str, Any]) -> dict[str, Any]: ...
    def evaluate_release_gates(
        self,
        *,
        candidate: dict[str, Any],
        target_environment: str = "live",
        experiment_registry_path: str | None = None,
    ) -> dict[str, Any]: ...
    def create_experiment(self, experiment: dict[str, Any]) -> dict[str, Any]: ...
    def delete_experiment(self, experiment_id: str) -> dict[str, Any]: ...
    def set_params(self, params: dict[str, Any]) -> dict[str, Any]: ...
    def set_system_mode(self, mode: str) -> dict[str, Any]: ...
    def set_system_power(self, power: str) -> dict[str, Any]: ...
    def set_execution_profile(
        self, *, execution_mode: str, apply_to_config_files: bool = True
    ) -> dict[str, Any]: ...
    def set_model_version(self, *, component: str, version: str) -> dict[str, Any]: ...
    def put_asset_model_manifest(self, symbol: str, manifest: dict[str, Any]) -> dict[str, Any]: ...
    def delete_asset_model_manifest(self, symbol: str) -> dict[str, Any]: ...
    def system_flatten(self) -> dict[str, Any]: ...


class HttpPlatformBackend:
    """:class:`PlatformBackend` backed by the FastAPI control plane over HTTP."""

    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8000",
        *,
        api_key: str | None = None,
        timeout: float = 15.0,
    ) -> None:
        self._base = base_url.rstrip("/")
        self._headers = {"X-API-Key": api_key} if api_key else {}
        self._timeout = timeout

    def _client(self) -> httpx.Client:
        return httpx.Client(base_url=self._base, headers=self._headers, timeout=self._timeout)

    def _get(self, path: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.get(path, params=params)
            r.raise_for_status()
            return r.json()

    def _post(self, path: str, json: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.post(path, json=json)
            r.raise_for_status()
            return r.json()

    def _put(self, path: str, json: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.put(path, json=json)
            r.raise_for_status()
            return r.json()

    def _delete(self, path: str) -> dict[str, Any]:
        with self._client() as c:
            r = c.delete(path)
            r.raise_for_status()
            return r.json()

    # --- reads ---
    def system_status(self) -> dict[str, Any]:
        return self._get("/status")

    def list_assets(self) -> dict[str, Any]:
        return self._get("/assets/models")

    def get_bars(
        self, symbol: str, *, limit: int = 200, interval_seconds: int | None = None
    ) -> dict[str, Any]:
        iv = interval_seconds or 60
        end = _dt.datetime.now(_dt.UTC)
        start = end - _dt.timedelta(seconds=iv * max(1, int(limit)))
        params: dict[str, Any] = {
            "symbol": symbol,
            "start": start.isoformat(),
            "end": end.isoformat(),
        }
        if interval_seconds:
            params["interval_seconds"] = int(interval_seconds)
        return self._get("/assets/chart/bars", params=params)

    def get_positions(self) -> dict[str, Any]:
        return self._get("/portfolio/positions")

    def get_pnl(self, *, window: str = "day") -> dict[str, Any]:
        return self._get("/pnl/summary", params={"range": window})

    def get_latest_decision(self) -> dict[str, Any]:
        return self._get("/governance/decision-record")

    # --- actions (mutating; carry operator key) ---
    def place_order(
        self,
        *,
        symbol: str,
        side: str,
        quantity: str,
        order_type: str = "market",
        limit_price: str | None = None,
        stop_price: str | None = None,
        time_in_force: str = "gtc",
        mid_price: float | None = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {
            "symbol": symbol,
            "side": side,
            "quantity": str(quantity),
            "order_type": order_type,
            "time_in_force": time_in_force,
        }
        if limit_price is not None:
            body["limit_price"] = str(limit_price)
        if stop_price is not None:
            body["stop_price"] = str(stop_price)
        if mid_price is not None:
            body["mid_price"] = float(mid_price)
        return self._post("/trade/order", json=body)

    def flatten(self, symbol: str) -> dict[str, Any]:
        return self._post("/trade/flatten", json={"symbol": symbol})

    def set_lifecycle(self, symbol: str, action: str) -> dict[str, Any]:
        a = action.strip().lower()
        if a == "initialize":
            return self._post(f"/assets/init/{symbol}")
        if a == "start":
            return self._post(f"/assets/lifecycle/{symbol}/start")
        if a == "stop":
            return self._post(f"/assets/lifecycle/{symbol}/stop")
        raise ValueError(f"invalid lifecycle action: {action!r} (use initialize|start|stop)")

    def set_execution_mode(self, symbol: str, mode: str) -> dict[str, Any]:
        m = mode.strip().lower()
        if m in ("default", "use default", ""):
            return self._delete(f"/assets/execution-mode/{symbol}")
        return self._put(f"/assets/execution-mode/{symbol}", json={"execution_mode": m})

    # --- extended reads ---
    def get_scheduler_status(self) -> dict[str, Any]:
        return self._get("/scheduler/nightly")

    def get_system_power(self) -> dict[str, Any]:
        return self._get("/system/power")

    def get_execution_profile(self) -> dict[str, Any]:
        return self._get("/system/execution-profile")

    def get_pnl_series(
        self, *, window: str = "day", bucket_seconds: int = 3600, mode: str | None = None
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"range": window, "bucket_seconds": int(bucket_seconds)}
        if mode:
            params["mode"] = mode
        return self._get("/pnl/series", params=params)

    def get_microservices_health(self) -> dict[str, Any]:
        return self._get("/microservices/health")

    def _universe_params(self, *, limit: int, offset: int, query: str | None) -> dict[str, Any]:
        params: dict[str, Any] = {"limit": int(limit), "offset": int(offset)}
        if query:
            params["q"] = query
        return params

    def list_alpaca_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]:
        return self._get("/universe/alpaca", params=self._universe_params(limit=limit, offset=offset, query=query))

    def list_coinbase_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]:
        return self._get("/universe/coinbase", params=self._universe_params(limit=limit, offset=offset, query=query))

    def list_platform_supported_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]:
        return self._get(
            "/universe/platform-supported", params=self._universe_params(limit=limit, offset=offset, query=query)
        )

    def search_universe(
        self, *, limit: int = 200, offset: int = 0, query: str | None = None
    ) -> dict[str, Any]:
        return self._get("/universe/search", params=self._universe_params(limit=limit, offset=offset, query=query))

    def get_routes(self) -> dict[str, Any]:
        return self._get("/routes")

    def get_params(self) -> dict[str, Any]:
        return self._get("/params")

    def get_system_mode(self) -> dict[str, Any]:
        return self._get("/system/mode")

    def list_models(self) -> dict[str, Any]:
        return self._get("/models")

    def get_asset_model_manifest(self, symbol: str) -> dict[str, Any]:
        return self._get(f"/assets/models/{symbol}")

    def get_asset_lifecycle(self, symbol: str) -> dict[str, Any]:
        return self._get(f"/assets/lifecycle/{symbol}")

    def get_asset_execution_mode(self, symbol: str) -> dict[str, Any]:
        return self._get(f"/assets/execution-mode/{symbol}")

    def get_asset_init_job(self, job_id: str) -> dict[str, Any]:
        return self._get(f"/assets/init/jobs/{job_id}")

    def get_latest_bar(self, symbol: str, *, interval_seconds: int | None = None) -> dict[str, Any]:
        params: dict[str, Any] = {"symbol": symbol}
        if interval_seconds:
            params["interval_seconds"] = int(interval_seconds)
        return self._get("/assets/chart/latest-bar", params=params)

    def get_trade_markers(
        self, symbol: str, *, start: str, end: str, limit: int = 2000
    ) -> dict[str, Any]:
        return self._get(
            "/assets/chart/trade-markers",
            params={"symbol": symbol, "start": start, "end": end, "limit": int(limit)},
        )

    def get_release_evidence(self) -> dict[str, Any]:
        return self._get("/governance/release-evidence")

    def get_governance_monitoring(self) -> dict[str, Any]:
        return self._get("/governance/monitoring")

    def get_probation_status(self) -> dict[str, Any]:
        return self._get("/governance/probation-status")

    def get_shadow_comparison(self) -> dict[str, Any]:
        return self._get("/governance/shadow-comparison")

    def get_config_diff_audit(self, *, limit: int = 50) -> dict[str, Any]:
        return self._get("/governance/config-diff-audit", params={"limit": int(limit)})

    def list_release_objects(self) -> dict[str, Any]:
        return self._get("/governance/release-objects")

    def get_release_object(self, release_id: str) -> dict[str, Any]:
        return self._get(f"/governance/release-objects/{release_id}")

    def get_rollback_playbook(self) -> dict[str, Any]:
        return self._get("/governance/rollback-playbook")

    def list_experiments(self, **filters: Any) -> dict[str, Any]:
        params = {k: v for k, v in filters.items() if v not in (None, "")}
        return self._get("/governance/experiments", params=params)

    def get_experiment(self, experiment_id: str) -> dict[str, Any]:
        return self._get(f"/governance/experiments/{experiment_id}")

    # --- extended actions (mutating; carry operator key) ---
    def sync_alpaca_universe(self) -> dict[str, Any]:
        return self._post("/universe/alpaca/sync")

    def sync_coinbase_universe(self) -> dict[str, Any]:
        return self._post("/universe/coinbase/sync")

    def run_shadow_comparison(self, **kwargs: Any) -> dict[str, Any]:
        body = {k: v for k, v in kwargs.items() if v is not None}
        return self._post("/governance/shadow-comparison/run", json=body)

    def diff_release_evidence(self, *, baseline_yaml: str, append_audit: bool = False) -> dict[str, Any]:
        return self._post(
            "/governance/release-evidence/diff",
            json={"baseline_yaml": baseline_yaml, "append_audit": bool(append_audit)},
        )

    def create_release_object(self, candidate: dict[str, Any]) -> dict[str, Any]:
        return self._post("/governance/release-objects", json=candidate)

    def evaluate_release_gates(
        self,
        *,
        candidate: dict[str, Any],
        target_environment: str = "live",
        experiment_registry_path: str | None = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"candidate": candidate, "target_environment": target_environment}
        if experiment_registry_path:
            body["experiment_registry_path"] = experiment_registry_path
        return self._post("/governance/release-objects/evaluate-gates", json=body)

    def create_experiment(self, experiment: dict[str, Any]) -> dict[str, Any]:
        return self._post("/governance/experiments", json=experiment)

    def delete_experiment(self, experiment_id: str) -> dict[str, Any]:
        return self._delete(f"/governance/experiments/{experiment_id}")

    def set_params(self, params: dict[str, Any]) -> dict[str, Any]:
        return self._post("/params", json=params)

    def set_system_mode(self, mode: str) -> dict[str, Any]:
        return self._post("/system/mode", json={"mode": mode})

    def set_system_power(self, power: str) -> dict[str, Any]:
        return self._post("/system/power", json={"power": power})

    def set_execution_profile(
        self, *, execution_mode: str, apply_to_config_files: bool = True
    ) -> dict[str, Any]:
        return self._post(
            "/system/execution-profile",
            json={"execution_mode": execution_mode, "apply_to_config_files": bool(apply_to_config_files)},
        )

    def set_model_version(self, *, component: str, version: str) -> dict[str, Any]:
        return self._post("/models/version", json={"component": component, "version": version})

    def put_asset_model_manifest(self, symbol: str, manifest: dict[str, Any]) -> dict[str, Any]:
        return self._put(f"/assets/models/{symbol}", json=manifest)

    def delete_asset_model_manifest(self, symbol: str) -> dict[str, Any]:
        return self._delete(f"/assets/models/{symbol}")

    def system_flatten(self) -> dict[str, Any]:
        return self._post("/flatten")

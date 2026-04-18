"""FastAPI control plane — /status, /routes, /params, /system/mode, /flatten, /models."""

from __future__ import annotations

import ipaddress
import logging
import os
import time
from collections import defaultdict
from datetime import UTC, datetime
from typing import Annotated, Any, Literal

from contextlib import asynccontextmanager

from pydantic import BaseModel, Field, ValidationError

from fastapi import Depends, FastAPI, HTTPException, Query, Request, Response
from fastapi import status as http_status
from fastapi.responses import PlainTextResponse, StreamingResponse
from fastapi.security import APIKeyHeader
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.middleware.cors import CORSMiddleware
from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

from decision_engine.decision_record import get_last_decision_record
from observability.drift_calibration_metrics import refresh_shadow_divergence_gauges_from_store
from observability.forecaster_metrics import MODEL_VERSION_INFO

from app.config.model_artifacts import model_artifact_contract
from app.config.settings import AppSettings, load_settings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.contracts.asset_lifecycle import AssetLifecycleState
from app.contracts.auth_login import AuthUserResponse, LoginRequest
from app.contracts.user_registration import RegisterRequest, RegisterResponse
from app.contracts.user_venue_credentials import VenueCredentialsPut, VenueCredentialsResponse
from app.runtime.asset_execution_mode import (
    delete_mode_override,
    list_mode_overrides,
    to_api_dict as asset_execution_mode_to_api_dict,
    write_mode_override,
)
from app.runtime.asset_lifecycle_state import (
    delete_lifecycle_state,
    effective_lifecycle_state,
    lifecycle_overview,
    state_dir as asset_lifecycle_state_dir,
    transition_start,
    transition_stop,
)
from app.runtime.asset_model_registry import (
    delete_manifest,
    list_symbols as list_asset_manifest_symbols,
    load_manifest,
    registry_dir,
    save_manifest,
)
from app.runtime import asset_execution_mode as asset_execution_mode_mod
from app.runtime import operator_sessions as operator_sessions_mod
from app.runtime import tenant_context as tenant_ctx
from app.runtime import user_store as user_store_mod
from app.runtime import user_venue_credentials as user_venue_credentials_mod
from app.runtime.auth_venue_status import venue_keys_status_for_user
from app.runtime.execution_settings_merge import merge_settings_for_execution
from app.runtime.user_store import UserRecord
from execution.adapter_registry import supported_adapters_for_settings
from control_plane.chart_bars import (
    query_canonical_bars_for_chart,
    query_latest_canonical_bar_for_chart,
)
from control_plane.chart_stream import sse_chart_bar_updates
from control_plane.preflight import preflight_report
from app.contracts.risk import SystemMode
from app.runtime.mode_manager import ModeManager
from app.runtime.state_manager import StateManager
from app.runtime.system_power import get_power, legacy_system_power_enabled, set_power
from control_plane.execution_profile import (
    apply_intent_to_config_files,
    legacy_execution_profile_api_enabled,
    profile_payload,
    status_execution_profile_section,
    write_pending_intent,
)
from control_plane.microservice_health import probe_microservices_health
from execution.flatten_stop import flatten_symbol_position_sync
from execution.pnl_summary import compute_pnl_series, compute_pnl_summary
from execution.portfolio_positions import fetch_portfolio_positions
from execution.service import ExecutionService
from execution.trade_markers import iter_markers, marker_to_api_dict
from app.runtime import alpaca_universe_store as alpaca_universe_store_mod
from app.runtime import coinbase_universe_store as coinbase_universe_store_mod
from app.runtime.platform_supported_universe import (
    platform_supported_payload,
    platform_supported_status_summary,
    universe_search_payload,
)
from orchestration.alpaca_universe_scheduler import (
    alpaca_universe_scheduler_status,
    start_alpaca_universe_scheduler,
    stop_alpaca_universe_scheduler,
)
from orchestration.alpaca_universe_sync import sync_alpaca_tradable_universe
from orchestration.coinbase_universe_scheduler import (
    coinbase_universe_scheduler_status,
    start_coinbase_universe_scheduler,
    stop_coinbase_universe_scheduler,
)
from orchestration.coinbase_universe_sync import sync_coinbase_tradable_universe
from orchestration.app_scheduler import (
    nightly_scheduler_detail,
    scheduler_status,
    start_app_background_scheduler,
    stop_app_background_scheduler,
)
from orchestration.asset_init_pipeline import get_job as get_init_job, try_start_asset_init_job
from app.contracts.release_objects import (
    ReleaseCandidate,
    ReleaseEnvironment,
    ReleaseLedger,
    read_release_ledger,
    write_release_ledger,
)
from orchestration.config_diff_audit import (
    append_config_diff_audit_entry,
    build_canonical_config_diff_report,
    read_config_diff_audit_tail,
)
from orchestration.release_evidence import (
    build_release_evidence_bundle,
    resolve_canonical_from_yaml_text,
)
from orchestration.release_gating import evaluate_promotion_gates
from orchestration.shadow_comparison import run_shadow_replay_pair_comparison
from app.config.shadow_comparison import shadow_policy_from_settings
from models.registry.shadow_comparison_store import (
    load_shadow_comparison_store,
    save_shadow_comparison_report,
)
from models.registry.experiment_registry import (
    ChangeType,
    ExperimentDomain,
    ExperimentRecord,
    ExperimentStatus,
    delete_experiment,
    load_or_create_experiment_registry,
    query_experiments,
    upsert_experiment,
    write_experiment_registry,
)

logger = logging.getLogger(__name__)

settings = load_settings()
state = StateManager()
modes = ModeManager(state)


def _control_plane_bind_loopback_only(host: str) -> bool:
    """True when the HTTP bind address is loopback-only (no API key warning needed)."""
    h = (host or "").strip().lower()
    if h in ("127.0.0.1", "localhost", "::1"):
        return True
    try:
        return ipaddress.ip_address(h).is_loopback
    except ValueError:
        return False


def _cors_allow_origins_list(s: AppSettings) -> list[str]:
    raw = (s.control_plane_cors_allow_origins or "").strip()
    if raw == "*" or not raw:
        return ["*"]
    return [x.strip() for x in raw.split(",") if x.strip()]


class _SlidingWindowRateLimiter:
    """Per-key sliding window (monotonic clock) for optional abuse protection."""

    def __init__(self, window_seconds: float = 60.0) -> None:
        self._window = float(window_seconds)
        self._hits: dict[str, list[float]] = defaultdict(list)

    def allow(self, key: str, max_requests: int) -> bool:
        cap = max(1, int(max_requests))
        now = time.monotonic()
        cutoff = now - self._window
        buf = self._hits[key]
        while buf and buf[0] < cutoff:
            buf.pop(0)
        if len(buf) >= cap:
            return False
        buf.append(now)
        return True


_rate_limiter = _SlidingWindowRateLimiter(60.0)


def _client_ip(request: Request) -> str:
    xf = request.headers.get("x-forwarded-for")
    if xf:
        return xf.split(",")[0].strip() or "unknown"
    if request.client:
        return request.client.host
    return "unknown"


class ControlPlaneRateLimitMiddleware(BaseHTTPMiddleware):
    """Optional per-IP request cap when ``NM_CONTROL_PLANE_RATE_LIMIT_ENABLED``."""

    async def dispatch(self, request: Request, call_next):  # type: ignore[no-untyped-def]
        if not settings.control_plane_rate_limit_enabled:
            return await call_next(request)
        if request.method == "OPTIONS":
            return await call_next(request)
        if not _rate_limiter.allow(
            _client_ip(request), settings.control_plane_rate_limit_per_minute
        ):
            raise HTTPException(
                status_code=http_status.HTTP_429_TOO_MANY_REQUESTS,
                detail="Rate limit exceeded",
            )
        return await call_next(request)


@asynccontextmanager
async def _lifespan(app: FastAPI):
    """FB-AP-035: register background schedulers only while this process runs."""
    if (
        settings.control_plane_api_key is None
        and not settings.auth_session_enabled
        and not _control_plane_bind_loopback_only(settings.control_plane_host)
    ):
        logger.warning(
            "control_plane: NM_CONTROL_PLANE_API_KEY is unset and session auth is off while "
            "bind host is %s — mutating routes are reachable without auth on the network. "
            "Set NM_CONTROL_PLANE_API_KEY, enable NM_AUTH_SESSION_ENABLED, or bind loopback-only.",
            settings.control_plane_host,
        )
    start_app_background_scheduler(settings)
    start_alpaca_universe_scheduler(settings)
    start_coinbase_universe_scheduler(settings)
    refresh_shadow_divergence_gauges_from_store(load_shadow_comparison_store())
    yield
    stop_coinbase_universe_scheduler()
    stop_alpaca_universe_scheduler()
    stop_app_background_scheduler()


class TenantContextMiddleware(BaseHTTPMiddleware):
    """FB-UX-007: bind ``tenant_ctx`` for the request (session → user id)."""

    async def dispatch(self, request: Request, call_next):  # type: ignore[no-untyped-def]
        uid: int | None = None
        if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
            if settings.auth_session_enabled:
                tok = request.cookies.get(settings.auth_session_cookie_name)
                if tok:
                    uid = operator_sessions_mod.resolve_session_user_id(
                        settings.auth_users_db_path, tok
                    )
        token = tenant_ctx.set_current_user_id_token(uid)
        try:
            return await call_next(request)
        finally:
            tenant_ctx.reset_current_user_id_token(token)


app = FastAPI(title="Trading Bot Control Plane", version="0.1.0", lifespan=_lifespan)
app.add_middleware(TenantContextMiddleware)
app.add_middleware(ControlPlaneRateLimitMiddleware)
# FB-AP-034: browser EventSource (Streamlit on another origin) needs CORS on SSE + JSON
_cors_origins = _cors_allow_origins_list(settings)
app.add_middleware(
    CORSMiddleware,
    allow_origins=_cors_origins,
    allow_credentials=bool(settings.auth_session_enabled and _cors_origins != ["*"]),
    allow_methods=["*"],
    allow_headers=["*"],
)

_api_key_header = APIKeyHeader(name="X-API-Key", auto_error=False)


def production_preflight_payload(s: AppSettings) -> dict[str, Any]:
    """Signing + venue credentials vs execution mode (IL-105 / FB-SPEC-08 complement)."""
    secret_val = (
        s.risk_signing_secret.get_secret_value().strip() if s.risk_signing_secret else ""
    )
    signing_configured = bool(secret_val)
    unsigned_allowed = s.allow_unsigned_execution
    alpaca_ok = bool(
        s.alpaca_api_key
        and s.alpaca_api_secret
        and s.alpaca_api_key.get_secret_value().strip()
        and s.alpaca_api_secret.get_secret_value().strip()
    )
    coinbase_ok = bool(
        s.coinbase_api_key
        and s.coinbase_api_secret
        and s.coinbase_api_key.get_secret_value().strip()
        and s.coinbase_api_secret.get_secret_value().strip()
    )
    venue_credentials_ok = alpaca_ok if s.execution_mode == "paper" else coinbase_ok
    ok = (
        signing_configured
        and not unsigned_allowed
        and venue_credentials_ok
    )
    return {
        "ok": ok,
        "signing_secret_configured": signing_configured,
        "unsigned_execution_allowed": unsigned_allowed,
        "venue_credentials_configured": venue_credentials_ok,
        "execution_mode": s.execution_mode,
    }


def require_mutate_key(x_api_key: Annotated[str | None, Depends(_api_key_header)]) -> None:
    """Mutating endpoints require NM_CONTROL_PLANE_API_KEY when set."""
    expected = (
        settings.control_plane_api_key.get_secret_value()
        if settings.control_plane_api_key
        else None
    )
    if not expected:
        return
    if x_api_key != expected:
        raise HTTPException(status_code=http_status.HTTP_401_UNAUTHORIZED, detail="Invalid or missing API key")


def get_current_user(request: Request) -> UserRecord | None:
    """Resolved from HTTP-only session cookie when ``NM_AUTH_SESSION_ENABLED`` (FB-UX-002)."""
    if not settings.auth_session_enabled:
        return None
    tok = request.cookies.get(settings.auth_session_cookie_name)
    uid = operator_sessions_mod.resolve_session_user_id(settings.auth_users_db_path, tok)
    if uid is None:
        return None
    return user_store_mod.get_user_by_id(settings.auth_users_db_path, uid)


def require_user(user: Annotated[UserRecord | None, Depends(get_current_user)]) -> UserRecord:
    """Future mutating routes that must be user-bound (not API-key automation)."""
    if user is None:
        raise HTTPException(status_code=http_status.HTTP_401_UNAUTHORIZED, detail="Not authenticated")
    return user


def _venue_credentials_master_secret_value() -> str | None:
    s = settings.auth_venue_credentials_master_secret
    if not s:
        return None
    v = s.get_secret_value().strip()
    return v or None


def require_mutate_operator(
    request: Request,
    x_api_key: Annotated[str | None, Depends(_api_key_header)],
) -> None:
    """Mutate: valid ``X-API-Key`` when configured, else valid session cookie when ``NM_AUTH_SESSION_ENABLED``."""
    expected = (
        settings.control_plane_api_key.get_secret_value()
        if settings.control_plane_api_key
        else None
    )
    if expected and x_api_key == expected:
        return
    if settings.auth_session_enabled:
        tok = request.cookies.get(settings.auth_session_cookie_name)
        if operator_sessions_mod.resolve_session_user_id(settings.auth_users_db_path, tok) is not None:
            return
        raise HTTPException(
            status_code=http_status.HTTP_401_UNAUTHORIZED,
            detail="Authentication required",
        )
    if not expected:
        return
    raise HTTPException(status_code=http_status.HTTP_401_UNAUTHORIZED, detail="Invalid or missing API key")


@app.get("/status")
def get_status() -> dict[str, Any]:
    return {
        "execution_mode": settings.execution_mode,
        "market_data_provider": settings.market_data_provider,
        "symbols": settings.market_data_symbols,
        "mode": modes.get_mode().value,
        "system_power": get_power(),
        "system_power_legacy_enabled": legacy_system_power_enabled(),
        "preflight": preflight_report(settings),
        "production_preflight": production_preflight_payload(settings),
        "model_artifacts": model_artifact_contract(settings),
        "execution_profile": status_execution_profile_section(settings),
        "execution_adapters": supported_adapters_for_settings(settings),
        "asset_model_registry": {
            "registry_dir": str(registry_dir()),
            "initialized_symbols": list_asset_manifest_symbols(),
        },
        "asset_lifecycle": {
            "state_dir": str(asset_lifecycle_state_dir()),
            "states": lifecycle_overview(),
        },
        "asset_execution_mode": {
            "default_execution_mode": settings.execution_mode,
            "sidecar_dir": str(asset_execution_mode_mod.mode_dir()),
            "overrides": list_mode_overrides(),
        },
        "app_scheduler": scheduler_status(),
        "alpaca_universe": {
            **alpaca_universe_store_mod.alpaca_universe_status(settings.alpaca_universe_db_path),
            **alpaca_universe_scheduler_status(),
        },
        "coinbase_universe": {
            **coinbase_universe_store_mod.coinbase_universe_status(settings.coinbase_universe_db_path),
            **coinbase_universe_scheduler_status(),
        },
        "platform_supported_universe": platform_supported_status_summary(settings),
        "user_store": {
            **user_store_mod.user_store_status(settings.auth_users_db_path),
            "session_auth_enabled": settings.auth_session_enabled,
            **operator_sessions_mod.session_status(settings.auth_users_db_path),
        },
    }


@app.get("/scheduler/nightly")
def get_nightly_scheduler_status() -> dict[str, Any]:
    """Nightly in-process scheduler: last/next run, last error, last training report (FB-UX-012)."""
    return nightly_scheduler_detail(settings)


@app.get("/universe/alpaca")
def get_alpaca_tradable_universe(
    limit: Annotated[int, Query(ge=1, le=10_000)] = 200,
    offset: Annotated[int, Query(ge=0)] = 0,
    q: Annotated[str | None, Query(description="Filter by symbol or name (case-insensitive)")] = None,
) -> dict[str, Any]:
    """Paginated Alpaca **tradable crypto** snapshot (FB-AP-020). Metadata only — no OHLC."""
    rows, total = alpaca_universe_store_mod.list_alpaca_universe_rows(
        settings.alpaca_universe_db_path,
        limit=limit,
        offset=offset,
        query=q,
    )
    return {
        "ok": True,
        "total": total,
        "limit": limit,
        "offset": offset,
        "query": q,
        "rows": rows,
    }


@app.post("/universe/alpaca/sync")
def post_alpaca_universe_sync(
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """On-demand refresh from Alpaca Trading API (mutating operator). FB-AP-020."""
    return sync_alpaca_tradable_universe(settings)


@app.get("/universe/coinbase")
def get_coinbase_tradable_universe(
    limit: Annotated[int, Query(ge=1, le=10_000)] = 200,
    offset: Annotated[int, Query(ge=0)] = 0,
    q: Annotated[str | None, Query(description="Filter by product_id or base name (case-insensitive)")] = None,
) -> dict[str, Any]:
    """Paginated Coinbase **SPOT** product snapshot (FB-AP-021). Metadata only — no OHLC."""
    rows, total = coinbase_universe_store_mod.list_coinbase_universe_rows(
        settings.coinbase_universe_db_path,
        limit=limit,
        offset=offset,
        query=q,
    )
    return {
        "ok": True,
        "total": total,
        "limit": limit,
        "offset": offset,
        "query": q,
        "rows": rows,
    }


@app.post("/universe/coinbase/sync")
def post_coinbase_universe_sync(
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """On-demand refresh from Coinbase Advanced Trade (mutating operator). FB-AP-021."""
    return sync_coinbase_tradable_universe(settings)


@app.get("/universe/platform-supported")
def get_platform_supported_universe(
    limit: Annotated[int, Query(ge=1, le=10_000)] = 200,
    offset: Annotated[int, Query(ge=0)] = 0,
    q: Annotated[str | None, Query(description="Filter symbol/name/base (case-insensitive)")] = None,
) -> dict[str, Any]:
    """Cross-venue **platform-supported** symbols (FB-AP-022). Search/eligibility only — not Kraken data."""
    return platform_supported_payload(settings, limit=limit, offset=offset, query=q)


@app.get("/universe/search")
def get_universe_search(
    limit: Annotated[int, Query(ge=1, le=10_000)] = 200,
    offset: Annotated[int, Query(ge=0)] = 0,
    q: Annotated[str | None, Query(description="Filter symbol/name/base (case-insensitive)")] = None,
) -> dict[str, Any]:
    """Paginated symbol search over the FB-AP-022 set with venue metadata only — no OHLC (FB-AP-023)."""
    return universe_search_payload(settings, limit=limit, offset=offset, query=q)


@app.post("/auth/register", response_model=RegisterResponse)
def post_register(body: RegisterRequest) -> RegisterResponse:
    """Create a user account (email + Argon2 password hash). FB-UX-001 — sessions in FB-UX-002."""
    try:
        rec = user_store_mod.create_user(settings.auth_users_db_path, body.email, body.password)
    except user_store_mod.InvalidEmailError as e:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(e),
        ) from e
    except user_store_mod.InvalidPasswordError as e:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(e),
        ) from e
    except user_store_mod.DuplicateEmailError as e:
        raise HTTPException(
            status_code=http_status.HTTP_409_CONFLICT,
            detail=str(e),
        ) from e
    req, ok = venue_keys_status_for_user(settings, rec.id)
    return RegisterResponse(
        id=rec.id,
        email=rec.email,
        created_at=rec.created_at,
        venue_keys_required=req,
        venue_keys_complete=ok,
    )


@app.post("/auth/login", response_model=AuthUserResponse)
def post_login(request: Request, response: Response, body: LoginRequest) -> AuthUserResponse:
    """Create server-side session and set HTTP-only cookie (FB-UX-002)."""
    if not settings.auth_session_enabled:
        raise HTTPException(
            status_code=http_status.HTTP_403_FORBIDDEN,
            detail="Session authentication is disabled (NM_AUTH_SESSION_ENABLED=false)",
        )
    rec = user_store_mod.verify_password(settings.auth_users_db_path, body.email, body.password)
    if rec is None:
        raise HTTPException(
            status_code=http_status.HTTP_401_UNAUTHORIZED,
            detail="Invalid email or password",
        )
    sess = operator_sessions_mod.create_session(
        settings.auth_users_db_path,
        rec.id,
        settings.auth_session_ttl_seconds,
    )
    response.set_cookie(
        key=settings.auth_session_cookie_name,
        value=sess.token,
        max_age=settings.auth_session_ttl_seconds,
        httponly=True,
        secure=settings.auth_session_cookie_secure,
        samesite=settings.auth_session_cookie_samesite,
        path="/",
    )
    req, ok = venue_keys_status_for_user(settings, rec.id)
    return AuthUserResponse(
        id=rec.id,
        email=rec.email,
        created_at=rec.created_at,
        venue_keys_required=req,
        venue_keys_complete=ok,
    )


@app.get("/auth/me", response_model=AuthUserResponse)
def get_me(user: Annotated[UserRecord, Depends(require_user)]) -> AuthUserResponse:
    """Current user from session cookie (requires ``NM_AUTH_SESSION_ENABLED``)."""
    req, ok = venue_keys_status_for_user(settings, user.id)
    return AuthUserResponse(
        id=user.id,
        email=user.email,
        created_at=user.created_at,
        venue_keys_required=req,
        venue_keys_complete=ok,
    )


@app.get("/auth/venue-credentials", response_model=VenueCredentialsResponse)
def get_venue_credentials(user: Annotated[UserRecord, Depends(require_user)]) -> VenueCredentialsResponse:
    """Masked Alpaca / Coinbase credential presence (FB-UX-006). Requires ``NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET``."""
    master = _venue_credentials_master_secret_value()
    if not master:
        raise HTTPException(
            status_code=http_status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Per-user venue credentials are not configured (set NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET)",
        )
    data = user_venue_credentials_mod.load_masked(settings.auth_users_db_path, master, user.id)
    return VenueCredentialsResponse(**data)


@app.put("/auth/venue-credentials", response_model=VenueCredentialsResponse)
def put_venue_credentials(
    body: VenueCredentialsPut,
    user: Annotated[UserRecord, Depends(require_user)],
) -> VenueCredentialsResponse:
    """Encrypt and store per-user venue API keys (FB-UX-006)."""
    master = _venue_credentials_master_secret_value()
    if not master:
        raise HTTPException(
            status_code=http_status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Per-user venue credentials are not configured (set NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET)",
        )

    def _nz(v: str | None) -> str | None:
        if v is None:
            return None
        s = str(v).strip()
        return s if s else None

    user_venue_credentials_mod.save_credentials(
        settings.auth_users_db_path,
        master,
        user.id,
        alpaca_api_key=_nz(body.alpaca_api_key),
        alpaca_api_secret=_nz(body.alpaca_api_secret),
        coinbase_api_key=_nz(body.coinbase_api_key),
        coinbase_api_secret=_nz(body.coinbase_api_secret),
        clear_alpaca=body.clear_alpaca,
        clear_coinbase=body.clear_coinbase,
    )
    data = user_venue_credentials_mod.load_masked(settings.auth_users_db_path, master, user.id)
    return VenueCredentialsResponse(**data)


@app.post("/auth/logout")
def post_logout(request: Request, response: Response) -> dict[str, bool]:
    """Revoke session server-side and clear cookie."""
    if not settings.auth_session_enabled:
        raise HTTPException(
            status_code=http_status.HTTP_403_FORBIDDEN,
            detail="Session authentication is disabled (NM_AUTH_SESSION_ENABLED=false)",
        )
    tok = request.cookies.get(settings.auth_session_cookie_name)
    if tok:
        operator_sessions_mod.revoke_session(settings.auth_users_db_path, tok)
    response.delete_cookie(settings.auth_session_cookie_name, path="/")
    return {"ok": True}


@app.get("/system/power")
def get_system_power() -> dict[str, Any]:
    """Legacy global power (FB-AP-039: disabled by default — always ``on`` unless ``NM_SYSTEM_POWER_LEGACY_ENABLED``)."""
    return {
        "power": get_power(),
        "legacy_enabled": legacy_system_power_enabled(),
    }


@app.get("/system/execution-profile")
def get_execution_profile() -> dict[str, Any]:
    """Legacy app-wide mode (FB-AP-040: disabled unless ``NM_EXECUTION_PROFILE_LEGACY_API``)."""
    if not legacy_execution_profile_api_enabled():
        raise HTTPException(
            status_code=http_status.HTTP_410_GONE,
            detail="App-wide execution profile API is disabled (FB-AP-040). Use per-asset "
            "PUT /assets/execution-mode/{symbol} or set NM_EXECUTION_PROFILE_LEGACY_API=true.",
        )
    return profile_payload(settings.execution_mode)


@app.post("/system/execution-profile")
def post_execution_profile(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Legacy: record operator intent (paper/live). Disabled by default (FB-AP-040)."""
    if not legacy_execution_profile_api_enabled():
        raise HTTPException(
            status_code=http_status.HTTP_410_GONE,
            detail="App-wide execution profile API is disabled (FB-AP-040). Use per-asset "
            "PUT /assets/execution-mode/{symbol} or set NM_EXECUTION_PROFILE_LEGACY_API=true.",
        )
    raw = str(body.get("execution_mode", "")).strip().lower()
    if raw not in ("paper", "live"):
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="execution_mode must be paper or live",
        )
    intent: Any = raw
    apply_files = bool(body.get("apply_to_config_files", True))
    files_updated: dict[str, bool] = {}
    if apply_files:
        files_updated = apply_intent_to_config_files(intent)
    write_pending_intent(intent, settings.execution_mode)
    out: dict[str, Any] = {
        **profile_payload(settings.execution_mode),
        "config_files_updated": files_updated,
        "note": "Restart control plane, live_service / live_service_app, and power_supervisor (or close and re-run run.bat) so all processes load the new mode.",
    }
    return out


@app.post("/system/power")
def post_system_power(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, str]:
    """Legacy global power (removed when ``NM_SYSTEM_POWER_LEGACY_ENABLED=false``)."""
    if not legacy_system_power_enabled():
        raise HTTPException(
            status_code=http_status.HTTP_410_GONE,
            detail="Global system power is disabled (FB-AP-039). Use per-asset Stop and process lifecycle; "
            "set NM_SYSTEM_POWER_LEGACY_ENABLED=true only if you need the legacy switch.",
        )
    raw = str(body.get("power", body.get("state", "on"))).strip().lower()
    p = "off" if raw in ("off", "false", "0") else "on"
    return {"power": set_power(p)}


@app.get("/portfolio/positions")
async def get_portfolio_positions() -> dict[str, Any]:
    """Open positions from the configured execution adapter (paper Alpaca / live Coinbase / stub)."""
    eff = merge_settings_for_execution(settings, tenant_ctx.get_current_user_id())
    return await fetch_portfolio_positions(eff)


@app.get("/pnl/summary")
async def get_pnl_summary(
    range_key: Annotated[
        Literal["hour", "day", "month", "year", "all"],
        Query(
            alias="range",
            description="Rolling window for realized P&L from local ledger (hour/day/month/year) or all time",
        ),
    ] = "day",
) -> dict[str, Any]:
    """Aggregate realized (local JSONL ledger) + unrealized (open positions) P&L. See docs/PNL_LEDGER.MD."""
    return await compute_pnl_summary(settings, range_key)  # merge inside pnl_summary uses tenant_ctx


@app.get("/pnl/series")
def get_pnl_series(
    range_key: Annotated[
        Literal["hour", "day", "month", "year", "all"],
        Query(
            alias="range",
            description="Same rolling windows as /pnl/summary",
        ),
    ] = "day",
    bucket_seconds: Annotated[
        int,
        Query(ge=60, le=86_400, description="Bucket width for ledger aggregation (seconds)"),
    ] = 3600,
) -> dict[str, Any]:
    """Cumulative realized P&L time series from the local ledger (FB-AP-026 dashboard chart)."""
    return compute_pnl_series(range_key, bucket_seconds=bucket_seconds)


@app.get("/microservices/health")
def microservices_health() -> dict[str, Any]:
    """Best-effort probes for optional scaffold processes (see infra/docker-compose.microservices.yml)."""
    return probe_microservices_health()


class ReleaseEvidenceDiffRequest(BaseModel):
    """Optional full default.yaml text to diff against the running merged canonical config."""

    baseline_yaml: str | None = Field(
        default=None,
        description="Full YAML document (e.g. app/config/default.yaml) for baseline canonical merge",
    )
    append_audit: bool = Field(
        default=False,
        description="If true, append full diff report to models/registry/config_diff_audit.jsonl (FB-CAN-057)",
    )


class ShadowComparisonRunRequest(BaseModel):
    """Run synthetic paired replay for shadow divergence metrics (FB-CAN-038)."""

    baseline_logic_version: str = "1.0.0"
    candidate_logic_version: str = "1.0.0"
    baseline_replay_run_id: str = "shadow-baseline"
    candidate_replay_run_id: str = "shadow-candidate"
    symbol: str = "BTC-USD"
    bars: int = Field(220, ge=50, le=5000)


class PromotionGateEvaluateRequest(BaseModel):
    """Evaluate promotion gates for a release candidate (FB-CAN-051)."""

    candidate: dict[str, Any]
    target_environment: ReleaseEnvironment = "live"
    experiment_registry_path: str | None = Field(
        default=None,
        description="Optional path to experiment_registry.json for linked_experiment_ids checks (FB-CAN-054).",
    )


@app.get("/governance/release-evidence")
def get_release_evidence() -> dict[str, Any]:
    """APEX release evidence bundle for the running process (FB-CAN-026)."""
    b = build_release_evidence_bundle()
    return b.model_dump(mode="json")


@app.get("/governance/decision-record")
def get_governance_decision_record() -> dict[str, Any]:
    """Last canonical DecisionRecord from this process (FB-CAN-036), if any."""
    rec = get_last_decision_record()
    if rec is None:
        return {"decision_record": None, "note": "no_decision_tick_yet"}
    return {"decision_record": rec}


@app.get("/governance/monitoring")
def get_governance_monitoring() -> dict[str, Any]:
    """Pointers to APEX canonical dashboards and Prometheus rules (FB-CAN-028)."""
    return {
        "docs": "docs/MONITORING_CANONICAL.MD",
        "spec": "docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md",
        "prometheus_rules": "infra/prometheus/alerts/canonical_apex.yml",
        "grafana_dashboard_uid": "tb-canonical-health",
        "metrics_module": "observability/canonical_metrics.py",
        "drift_calibration_metrics": "observability/drift_calibration_metrics.py",
        "carry_sleeve_metrics": (
            "tb_canonical_carry_sleeve_active, tb_canonical_carry_target_notional_usd, "
            "tb_canonical_carry_funding_signal, tb_canonical_carry_trigger_confidence, "
            "tb_canonical_carry_decision_quality, tb_canonical_carry_reason_total, "
            "tb_canonical_carry_directional_suppression_total (FB-CAN-064)"
        ),
        "governance_operator_metrics": (
            "tb_governance_promotion_attempt, tb_governance_gate_outcome, tb_governance_gate_failure, "
            "tb_governance_config_drift_event, tb_governance_rollback_event (FB-CAN-065)"
        ),
        "governance_metrics_module": "observability/governance_metrics.py",
    }


@app.get("/governance/shadow-comparison")
def get_governance_shadow_comparison() -> dict[str, Any]:
    """Last persisted shadow vs baseline replay comparison + policy (FB-CAN-038)."""
    st = load_shadow_comparison_store()
    pol = shadow_policy_from_settings(settings)
    return {
        "policy": pol.model_dump(mode="json"),
        "store": st,
        "docs": "docs/GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD",
    }


@app.post("/governance/shadow-comparison/run")
def post_governance_shadow_comparison_run(
    body: ShadowComparisonRunRequest,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Run paired replays, persist structured report, return JSON (operator tooling)."""
    rep = run_shadow_replay_pair_comparison(
        settings=settings,
        bars=int(body.bars),
        symbol=body.symbol.strip() or "BTC-USD",
        baseline_replay_run_id=body.baseline_replay_run_id.strip() or "shadow-baseline",
        candidate_replay_run_id=body.candidate_replay_run_id.strip() or "shadow-candidate",
        baseline_logic_version=body.baseline_logic_version.strip() or "1.0.0",
        candidate_logic_version=body.candidate_logic_version.strip() or "1.0.0",
    )
    save_shadow_comparison_report(rep)
    refresh_shadow_divergence_gauges_from_store(load_shadow_comparison_store())
    return rep


@app.post("/governance/release-evidence/diff")
def post_release_evidence_diff(body: ReleaseEvidenceDiffRequest) -> dict[str, Any]:
    """Structured diff between baseline YAML and the running merged canonical config."""
    if not (body.baseline_yaml and body.baseline_yaml.strip()):
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="baseline_yaml is required",
        )
    baseline = resolve_canonical_from_yaml_text(body.baseline_yaml)
    current = settings.canonical
    report = build_canonical_config_diff_report(baseline, current)
    try:
        from observability.governance_metrics import record_config_diff_report  # noqa: PLC0415

        record_config_diff_report(report)
    except Exception:
        pass
    if body.append_audit:
        append_config_diff_audit_entry(report)
    return report


@app.get("/governance/config-diff-audit")
def get_config_diff_audit(limit: Annotated[int, Query(ge=1, le=500)] = 50) -> dict[str, Any]:
    """Last N immutable config diff audit entries (JSONL tail; FB-CAN-057)."""
    rows = read_config_diff_audit_tail(limit=limit)
    return {"entries": rows, "count": len(rows)}


@app.get("/governance/release-objects")
def list_release_objects() -> dict[str, Any]:
    """APEX release ledger (FB-CAN-051): config/logic/model/feature/combined release records."""
    led = read_release_ledger()
    if led is None:
        return {"schema_version": 1, "candidates": [], "count": 0, "ledger_path": "models/registry/release_ledger.json"}
    d = led.model_dump(mode="json")
    d["count"] = len(led.candidates)
    return d


@app.get("/governance/release-objects/{release_id}")
def get_release_object(release_id: str) -> dict[str, Any]:
    """Return one release candidate by ``release_id``."""
    led = read_release_ledger()
    if led is None:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="release ledger not found")
    for c in led.candidates:
        if c.release_id == release_id:
            return c.model_dump(mode="json")
    raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="release_id not found")


@app.post("/governance/release-objects")
def post_release_object(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Create or replace a release candidate by ``release_id`` in the ledger file."""
    cand = ReleaseCandidate.model_validate(body)
    led = read_release_ledger()
    if led is None:
        led = ReleaseLedger()
    rest: list[ReleaseCandidate] = [c for c in led.candidates if c.release_id != cand.release_id]
    rest.append(cand)
    led = ReleaseLedger(candidates=rest)
    write_release_ledger(led)
    try:
        from observability.governance_metrics import record_rollback_release_candidate  # noqa: PLC0415

        if str(cand.current_stage) == "rolled_back":
            record_rollback_release_candidate(source="release_object_write")
    except Exception:
        pass
    return {"ok": True, "release_id": cand.release_id}


@app.post("/governance/release-objects/evaluate-gates")
def post_release_objects_evaluate_gates(body: PromotionGateEvaluateRequest) -> dict[str, Any]:
    """Evaluate promotion gates for a candidate JSON (no persistence)."""
    cand = ReleaseCandidate.model_validate(body.candidate)
    kwargs: dict[str, Any] = {"target_environment": body.target_environment}
    if body.experiment_registry_path:
        kwargs["experiment_registry_path"] = body.experiment_registry_path
    result = evaluate_promotion_gates(cand, **kwargs)
    return result.model_dump(mode="json")


@app.get("/governance/rollback-playbook")
def get_governance_rollback_playbook() -> dict[str, Any]:
    """Rollback playbook requirements and pointers (FB-CAN-053)."""
    return {
        "docs": "docs/operations/rollback_playbooks.md",
        "spec": "docs/Human Provided Specs/new_specs/canonical/APEX_Config_Management_and_Release_Gating_Spec_v1_0.md",
        "ci_script": "scripts/ci_rollback_playbook.sh",
        "release_objects_api": "GET /governance/release-objects",
        "requirements": {
            "research": {"rollback_playbook_text_required": False},
            "simulation": {
                "rollback_playbook_text_required": True,
                "fields": ["rollback_owner", "instructions", "trigger_conditions"],
            },
            "shadow": {
                "rollback_playbook_text_required": True,
                "fields": ["rollback_owner", "instructions", "trigger_conditions"],
            },
            "live": {
                "rollback_playbook_text_required": True,
                "fields": ["rollback_owner", "instructions", "trigger_conditions"],
            },
        },
        "validate_fn": "orchestration.rollback_validation.validate_rollback_playbook",
    }


@app.get("/governance/experiments")
def list_experiments(
    domain: Annotated[ExperimentDomain | None, Query()] = None,
    status: Annotated[ExperimentStatus | None, Query()] = None,
    change_type: Annotated[ChangeType | None, Query()] = None,
    tag: Annotated[str | None, Query()] = None,
    linked_release: Annotated[str | None, Query()] = None,
    notes_substring: Annotated[str | None, Query()] = None,
) -> dict[str, Any]:
    """APEX experiment registry (FB-CAN-027): filterable list."""
    reg = load_or_create_experiment_registry()
    rows = query_experiments(
        reg,
        domain=domain,
        status=status,
        change_type=change_type,
        tag=tag,
        linked_release=linked_release,
        notes_substring=notes_substring,
    )
    return {"experiments": [e.model_dump(mode="json") for e in rows], "count": len(rows)}


@app.get("/governance/experiments/{experiment_id}")
def get_experiment(experiment_id: str) -> dict[str, Any]:
    reg = load_or_create_experiment_registry()
    for e in reg.experiments:
        if e.experiment_id == experiment_id:
            return e.model_dump(mode="json")
    raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="experiment not found")


@app.post("/governance/experiments")
def post_experiment(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Create or replace an experiment by ``experiment_id`` (JSON body matches ``ExperimentRecord``)."""
    try:
        rec = ExperimentRecord.model_validate(body)
        reg = load_or_create_experiment_registry()
        new_reg = upsert_experiment(reg, rec)
        write_experiment_registry(new_reg)
    except ValidationError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=exc.errors(),
        ) from exc
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(exc),
        ) from exc
    return rec.model_dump(mode="json")


@app.delete("/governance/experiments/{experiment_id}")
def remove_experiment(
    experiment_id: str,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, str]:
    try:
        reg = load_or_create_experiment_registry()
        new_reg = delete_experiment(reg, experiment_id)
        write_experiment_registry(new_reg)
    except KeyError:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="experiment not found") from None
    return {"deleted": experiment_id}


@app.get("/routes")
def routes() -> dict[str, list[str]]:
    return {"routes": ["NO_TRADE", "SCALPING", "INTRADAY", "SWING"]}


@app.get("/params")
def params() -> dict[str, Any]:
    return state.get_params()


@app.post("/params")
def set_params(
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    state.set_params(body)
    return state.get_params()


@app.get("/system/mode")
def get_mode() -> dict[str, str]:
    return {"mode": modes.get_mode().value}


@app.post("/system/mode")
def set_mode(
    body: dict[str, str],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, str]:
    m = SystemMode(body.get("mode", "RUNNING"))
    modes.set_mode(m)
    return {"mode": modes.get_mode().value}


@app.post("/flatten")
def flatten(_: Annotated[None, Depends(require_mutate_operator)]) -> dict[str, str]:
    modes.set_mode(SystemMode.FLATTEN_ALL)
    return {"mode": modes.get_mode().value, "note": "flatten requested — execution layer must honor"}


@app.get("/models")
def models() -> dict[str, list[str]]:
    return {
        "models": [
            "forecaster_xlstm_reference",
            "policy_system_mlp_heuristic",
            "risk_engine_hmac",
        ]
    }


@app.post("/models/version")
def set_model_version(
    body: dict[str, str],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, str]:
    """Expose model version labels to Prometheus (FB-PL-PG7)."""
    component = body.get("component", "forecaster")
    version = body.get("version", "unknown")
    MODEL_VERSION_INFO.labels(component=component, version=version).set(1)
    return {"component": component, "version": version}


@app.get("/metrics", response_class=PlainTextResponse)
def metrics() -> PlainTextResponse:
    return PlainTextResponse(generate_latest(), media_type=CONTENT_TYPE_LATEST)


@app.get("/assets/models")
def list_asset_model_manifests() -> dict[str, Any]:
    """List symbols with a persisted per-asset model manifest (FB-AP-002)."""
    return {
        "registry_dir": str(registry_dir()),
        "symbols": list_asset_manifest_symbols(),
    }


@app.get("/assets/models/{symbol}")
def get_asset_model_manifest(symbol: str) -> dict[str, Any]:
    """Load manifest for ``symbol``; 404 if missing."""
    m = load_manifest(symbol)
    if m is None:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="no manifest")
    return m.model_dump(mode="json")


@app.put("/assets/models/{symbol}")
def put_asset_model_manifest(
    symbol: str,
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Create or replace manifest; path ``symbol`` must match body ``canonical_symbol`` (FB-AP-001)."""
    m = AssetModelManifest.model_validate(body)
    if m.canonical_symbol.strip() != symbol.strip():
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="canonical_symbol must match path symbol",
        )
    path = save_manifest(m)
    return {"ok": True, "path": str(path), "manifest": m.model_dump(mode="json")}


@app.delete("/assets/models/{symbol}")
def remove_asset_model_manifest(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Remove manifest file for ``symbol`` if present."""
    ok = delete_manifest(symbol)
    if not ok:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="no manifest")
    delete_lifecycle_state(symbol)
    return {"ok": True, "symbol": symbol.strip()}


@app.post("/assets/init/{symbol}")
def post_asset_init(request: Request, symbol: str) -> dict[str, Any]:
    """
    Start per-asset initialization (FB-AP-006): Kraken REST bootstrap, validate, enrich features
    (FB-AP-009 writes Parquet under ``data/asset_init``). One global runner — 409 if busy.
    """
    sym = symbol.strip()
    uid: int | None = None
    if os.getenv("NM_MULTI_TENANT_DATA_SCOPING", "").strip().lower() in ("1", "true", "yes"):
        if settings.auth_session_enabled:
            tok = request.cookies.get(settings.auth_session_cookie_name)
            if tok:
                uid = operator_sessions_mod.resolve_session_user_id(settings.auth_users_db_path, tok)
    job_id = try_start_asset_init_job(sym, user_id=uid)
    if job_id is None:
        raise HTTPException(
            status_code=http_status.HTTP_409_CONFLICT,
            detail="asset init pipeline already running",
        )
    return {"job_id": job_id, "symbol": sym}


@app.get("/assets/init/jobs/{job_id}")
def get_asset_init_job(job_id: str) -> dict[str, Any]:
    """Poll init job status and per-step detail (FB-AP-006)."""
    j = get_init_job(job_id)
    if j is None:
        raise HTTPException(status_code=http_status.HTTP_404_NOT_FOUND, detail="unknown job_id")
    return j


@app.get("/assets/lifecycle/{symbol}")
def get_asset_lifecycle(symbol: str) -> dict[str, Any]:
    """Per-asset lifecycle state for UI buttons: Initialize / Start / Stop (FB-AP-005)."""
    sym = symbol.strip()
    return {
        "symbol": sym,
        "lifecycle_state": effective_lifecycle_state(sym).value,
    }


@app.get("/assets/execution-mode/{symbol}")
def get_asset_execution_mode(symbol: str) -> dict[str, Any]:
    """Per-symbol paper/live routing for orders (FB-AP-030); falls back to default when unset."""
    sym = symbol.strip()
    return asset_execution_mode_to_api_dict(sym, settings)


@app.put("/assets/execution-mode/{symbol}")
def put_asset_execution_mode(
    symbol: str,
    body: dict[str, Any],
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Persist paper vs live for this symbol (overrides ``NM_EXECUTION_MODE`` for its orders)."""
    sym = symbol.strip()
    mode = body.get("execution_mode")
    if mode not in ("paper", "live"):
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="execution_mode must be 'paper' or 'live'",
        )
    path = write_mode_override(sym, mode)
    return {
        "ok": True,
        "symbol": sym,
        "execution_mode": mode,
        "path": str(path),
    }


@app.delete("/assets/execution-mode/{symbol}")
def delete_asset_execution_mode(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Remove per-symbol override; routing uses application default again."""
    sym = symbol.strip()
    ok = delete_mode_override(sym)
    if not ok:
        raise HTTPException(
            status_code=http_status.HTTP_404_NOT_FOUND,
            detail="no execution_mode sidecar for symbol",
        )
    return {"ok": True, "symbol": sym}


@app.post("/assets/lifecycle/{symbol}/start")
def post_asset_lifecycle_start(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """Start watch: ``initialized_not_active`` → ``active`` (requires manifest)."""
    sym = symbol.strip()
    try:
        path = transition_start(sym)
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail=str(exc),
        ) from exc
    return {
        "ok": True,
        "symbol": sym,
        "lifecycle_state": AssetLifecycleState.active.value,
        "path": str(path),
    }


@app.get("/assets/chart/bars")
async def get_asset_chart_bars(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    start: Annotated[datetime, Query(description="Range start (UTC ISO-8601)")],
    end: Annotated[datetime, Query(description="Range end (UTC ISO-8601)")],
    interval_seconds: Annotated[
        int | None,
        Query(
            description="Bar width in seconds; default = NM_MARKET_DATA_BAR_INTERVAL_SECONDS",
        ),
    ] = None,
    limit: Annotated[int, Query(ge=1, le=50_000, description="Max rows returned")] = 5000,
) -> dict[str, Any]:
    """Symbol-scoped OHLCV from QuestDB ``canonical_bars`` for chart use (FB-AP-024)."""
    try:
        return await query_canonical_bars_for_chart(
            settings,
            symbol=symbol,
            start=start,
            end=end,
            interval_seconds=interval_seconds,
            limit=limit,
        )
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(exc),
        ) from exc


@app.get("/assets/chart/latest-bar")
async def get_asset_chart_latest_bar(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    interval_seconds: Annotated[
        int | None,
        Query(
            description="Bar width in seconds; default = NM_MARKET_DATA_BAR_INTERVAL_SECONDS",
        ),
    ] = None,
) -> dict[str, Any]:
    """Latest stored canonical bar for last-price / SSE alignment (FB-AP-034)."""
    try:
        bar = await query_latest_canonical_bar_for_chart(
            settings,
            symbol=symbol,
            interval_seconds=interval_seconds,
        )
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail=str(exc),
        ) from exc
    if bar is None:
        raise HTTPException(
            status_code=http_status.HTTP_404_NOT_FOUND,
            detail="no canonical bar for symbol and interval",
        )
    return {"ok": True, "bar": bar}


@app.get("/assets/chart/stream")
async def get_asset_chart_stream(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    interval_seconds: Annotated[
        int | None,
        Query(
            description="Bar width in seconds; default = NM_MARKET_DATA_BAR_INTERVAL_SECONDS",
        ),
    ] = None,
    poll_seconds: Annotated[
        float,
        Query(ge=0.5, le=60.0, description="Server poll interval for new bars"),
    ] = 2.0,
) -> StreamingResponse:
    """Server-Sent Events: new canonical bar when QuestDB ``ts`` advances (FB-AP-034)."""
    return StreamingResponse(
        sse_chart_bar_updates(
            settings,
            symbol=symbol,
            interval_seconds=interval_seconds,
            poll_seconds=poll_seconds,
        ),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
        },
    )


@app.get("/assets/chart/trade-markers")
def get_asset_chart_trade_markers(
    symbol: Annotated[str, Query(min_length=1, description="Canonical symbol (e.g. BTC-USD)")],
    start: Annotated[datetime, Query(description="Range start (UTC ISO-8601)")],
    end: Annotated[datetime, Query(description="Range end (UTC ISO-8601)")],
    limit: Annotated[int, Query(ge=1, le=10_000, description="Max markers returned")] = 2000,
) -> dict[str, Any]:
    """Symbol-scoped buy/sell markers from append-only ``data/trade_markers.jsonl`` (FB-AP-025)."""
    sym = symbol.strip()
    s = start.replace(tzinfo=UTC) if start.tzinfo is None else start.astimezone(UTC)
    e = end.replace(tzinfo=UTC) if end.tzinfo is None else end.astimezone(UTC)
    if s >= e:
        raise HTTPException(
            status_code=http_status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="start must be before end",
        )
    rows = iter_markers(symbol=sym, start=s, end=e)
    lim = min(int(limit), 10_000)
    if len(rows) > lim:
        rows = rows[:lim]
    return {
        "symbol": sym,
        "start": s.isoformat(),
        "end": e.isoformat(),
        "limit": lim,
        "count": len(rows),
        "source": "trade_markers_jsonl",
        "markers": [marker_to_api_dict(m) for m in rows],
    }


@app.post("/assets/lifecycle/{symbol}/stop")
def post_asset_lifecycle_stop(
    symbol: str,
    _: Annotated[None, Depends(require_mutate_operator)],
) -> dict[str, Any]:
    """
    Stop watch: flatten open venue position for this symbol (market close), then
    ``active`` → ``initialized_not_active`` (FB-AP-032).
    """
    sym = symbol.strip()
    cur = effective_lifecycle_state(sym)
    if cur != AssetLifecycleState.active:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail="not active",
        )
    eff = merge_settings_for_execution(settings, tenant_ctx.get_current_user_id())
    exec_svc = ExecutionService(eff)
    flatten_report = flatten_symbol_position_sync(eff, sym, execution_service=exec_svc)
    if not flatten_report.get("lifecycle_continue", True):
        raise HTTPException(
            status_code=http_status.HTTP_502_BAD_GATEWAY,
            detail={
                "message": "flatten failed or could not verify position — lifecycle left active",
                "flatten": flatten_report,
            },
        )
    try:
        path = transition_stop(sym)
    except ValueError as exc:
        raise HTTPException(
            status_code=http_status.HTTP_400_BAD_REQUEST,
            detail=str(exc),
        ) from exc
    return {
        "ok": True,
        "symbol": sym,
        "lifecycle_state": AssetLifecycleState.initialized_not_active.value,
        "path": str(path),
        "flatten": flatten_report,
    }

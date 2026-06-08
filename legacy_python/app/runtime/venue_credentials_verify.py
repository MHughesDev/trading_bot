"""Live-ping verification for per-user Alpaca / Coinbase API credentials.

Pings each venue's REST API with the entered key + secret so misconfigured or
flat-out wrong credentials are caught at save time rather than failing silently
later during live trading. Best-effort diagnoses whether the *key* or the
*secret* is the likely culprit so the UI can point at the right field.
"""

from __future__ import annotations

import asyncio
import re
from dataclasses import dataclass
from typing import Awaitable, Callable

import httpx
from coinbase import jwt_generator

ALPACA_ACCOUNT_URL = "https://paper-api.alpaca.markets/v2/account"
COINBASE_ACCOUNTS_PATH = "/api/v3/brokerage/accounts"

_ALPACA_KEY_RE = re.compile(r"^[A-Za-z0-9]{16,32}$")
_COINBASE_KEY_RE = re.compile(r"^organizations/[^/]+/apiKeys/[^/]+$")

_TIMEOUT_SECONDS = 10.0


@dataclass
class VerifyResult:
    ok: bool
    key_error: str | None = None
    secret_error: str | None = None
    error: str | None = None


async def verify_alpaca(api_key: str, api_secret: str) -> VerifyResult:
    """Ping the Alpaca paper-trading account endpoint with the given key/secret."""
    key = api_key.strip()
    secret = api_secret.strip()
    key_looks_off = not _ALPACA_KEY_RE.match(key)
    secret_looks_off = len(secret) < 30

    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT_SECONDS) as client:
            resp = await client.get(
                ALPACA_ACCOUNT_URL,
                headers={"APCA-API-KEY-ID": key, "APCA-API-SECRET-KEY": secret},
            )
    except httpx.HTTPError as exc:
        return VerifyResult(ok=False, error=f"Could not reach Alpaca to verify these keys ({exc})")

    if resp.status_code == 200:
        return VerifyResult(ok=True)
    if resp.status_code in (401, 403):
        if key_looks_off and not secret_looks_off:
            return VerifyResult(ok=False, key_error="Doesn't look like a valid Alpaca API key")
        if secret_looks_off and not key_looks_off:
            return VerifyResult(ok=False, secret_error="Doesn't look like a valid Alpaca secret key")
        if key_looks_off and secret_looks_off:
            return VerifyResult(
                ok=False,
                key_error="Doesn't look like a valid Alpaca API key",
                secret_error="Doesn't look like a valid Alpaca secret key",
            )
        return VerifyResult(
            ok=False,
            error="Alpaca rejected this API key and secret combination — double-check both values",
        )
    return VerifyResult(ok=False, error=f"Alpaca returned an unexpected response (HTTP {resp.status_code})")


async def verify_coinbase(api_key: str, api_secret: str) -> VerifyResult:
    """Ping the Coinbase Advanced Trade accounts endpoint with the given CDP key/secret."""
    key = api_key.strip()
    secret = api_secret.strip()

    key_error = (
        None
        if _COINBASE_KEY_RE.match(key)
        else "Doesn't look like a Coinbase CDP key name (expected organizations/.../apiKeys/...)"
    )
    secret_error = (
        None
        if "PRIVATE KEY" in secret.upper()
        else "Doesn't look like a Coinbase private key (expected a PEM block: -----BEGIN EC PRIVATE KEY-----)"
    )
    if key_error or secret_error:
        return VerifyResult(ok=False, key_error=key_error, secret_error=secret_error)

    try:
        uri = jwt_generator.format_jwt_uri("GET", COINBASE_ACCOUNTS_PATH)
        token = jwt_generator.build_rest_jwt(uri, key, secret)
    except Exception as exc:
        return VerifyResult(ok=False, secret_error=f"Could not build a request from this private key ({exc})")

    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT_SECONDS) as client:
            resp = await client.get(
                f"https://api.coinbase.com{COINBASE_ACCOUNTS_PATH}",
                headers={"Authorization": f"Bearer {token}", "Accept": "application/json"},
            )
    except httpx.HTTPError as exc:
        return VerifyResult(ok=False, error=f"Could not reach Coinbase to verify these keys ({exc})")

    if resp.status_code == 200:
        return VerifyResult(ok=True)
    if resp.status_code in (401, 403):
        return VerifyResult(
            ok=False,
            error="Coinbase rejected this API key and private key combination — double-check both values",
        )
    return VerifyResult(ok=False, error=f"Coinbase returned an unexpected response (HTTP {resp.status_code})")


@dataclass(frozen=True)
class VenueSpec:
    """Registration record describing how to live-verify one venue's credentials.

    Adding support for a new venue is just appending a `VenueSpec` to
    `VENUE_REGISTRY` once a `verify_<venue>(api_key, api_secret) -> VerifyResult`
    coroutine exists for it — no endpoint or handler rewiring required.

    `verifier_name` (rather than a direct function reference) is the *name* of the
    module-level coroutine to call, resolved dynamically at verification time via
    `globals()`. This indirection is what lets tests (and any future runtime
    swap-out) monkeypatch `verify_alpaca` / `verify_coinbase` on this module and
    have `verify_venues` actually pick up the replacement.
    """

    name: str
    key_field: str
    secret_field: str
    verifier_name: str


VENUE_REGISTRY: tuple[VenueSpec, ...] = (
    VenueSpec("alpaca", "alpaca_api_key", "alpaca_api_secret", "verify_alpaca"),
    VenueSpec("coinbase", "coinbase_api_key", "coinbase_api_secret", "verify_coinbase"),
    # Register future venues here, e.g.:
    # VenueSpec("webull", "webull_api_key", "webull_api_secret", "verify_webull"),
)

VENUE_SPECS_BY_NAME: dict[str, VenueSpec] = {spec.name: spec for spec in VENUE_REGISTRY}


def _resolve_verifier(spec: VenueSpec) -> Callable[[str, str], Awaitable[VerifyResult]]:
    fn = globals().get(spec.verifier_name)
    if fn is None:
        raise RuntimeError(f"No verifier function named {spec.verifier_name!r} is defined for venue {spec.name!r}")
    return fn


async def verify_venues(requests: dict[str, tuple[str, str]]) -> dict[str, VerifyResult]:
    """Run live verification for several venues concurrently.

    `requests` maps venue name -> (api_key, api_secret). Only venues that are both
    registered in `VENUE_REGISTRY` and present in `requests` are checked. Any
    unexpected exception from a verifier is converted into a failed `VerifyResult`
    rather than propagating, so one venue's crash can't block the others — this is
    the single handler that all venues (current and future) funnel through, so a
    user can never end up "Connected" without a credential pair that actually
    passed a live ping.
    """
    names = [name for name in requests if name in VENUE_SPECS_BY_NAME]
    results = await asyncio.gather(
        *(_resolve_verifier(VENUE_SPECS_BY_NAME[name])(*requests[name]) for name in names),
        return_exceptions=True,
    )
    out: dict[str, VerifyResult] = {}
    for name, res in zip(names, results):
        if isinstance(res, BaseException):
            out[name] = VerifyResult(ok=False, error=f"Verification crashed: {res}")
        else:
            out[name] = res
    return out

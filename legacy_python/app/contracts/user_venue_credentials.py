"""HTTP models for per-user venue API credentials (FB-UX-006)."""

from __future__ import annotations

from pydantic import BaseModel


class VenueCredentialsPut(BaseModel):
    """Omit or send empty string to leave unchanged; use clear_* to remove."""

    alpaca_api_key: str | None = None
    alpaca_api_secret: str | None = None
    coinbase_api_key: str | None = None
    coinbase_api_secret: str | None = None
    webull_api_key: str | None = None
    webull_api_secret: str | None = None
    clear_alpaca: bool = False
    clear_coinbase: bool = False
    clear_webull: bool = False


class VenueCredentialsVerifyRequest(BaseModel):
    """Live-ping check for the key/secret pair about to be saved (FB-UX-006 hardening)."""

    alpaca_api_key: str | None = None
    alpaca_api_secret: str | None = None
    coinbase_api_key: str | None = None
    coinbase_api_secret: str | None = None


class VenueVerifyResult(BaseModel):
    ok: bool
    key_error: str | None = None
    secret_error: str | None = None
    error: str | None = None


class VenueCredentialsVerifyResponse(BaseModel):
    alpaca: VenueVerifyResult | None = None
    coinbase: VenueVerifyResult | None = None


class VenueCredentialsResponse(BaseModel):
    alpaca_key_set: bool
    alpaca_secret_set: bool
    coinbase_key_set: bool
    coinbase_secret_set: bool
    webull_key_set: bool = False
    webull_secret_set: bool = False
    alpaca_key_masked: str | None = None
    alpaca_secret_masked: str | None = None
    coinbase_key_masked: str | None = None
    coinbase_secret_masked: str | None = None
    webull_key_masked: str | None = None
    webull_secret_masked: str | None = None
    updated_at: str | None = None

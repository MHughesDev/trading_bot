"""Whether venue API credentials are present for the effective execution mode (env-based)."""

from __future__ import annotations

from app.config.settings import AppSettings


def venue_credentials_configured(settings: AppSettings) -> bool:
    """Return True if env has keys for the configured execution mode's venue adapter."""
    mode = settings.execution_mode
    if mode == "live":
        return bool(
            settings.coinbase_api_key
            and settings.coinbase_api_secret
            and settings.coinbase_api_key.get_secret_value().strip()
            and settings.coinbase_api_secret.get_secret_value().strip()
        )
    # paper
    return bool(
        settings.alpaca_api_key
        and settings.alpaca_api_secret
        and settings.alpaca_api_key.get_secret_value().strip()
        and settings.alpaca_api_secret.get_secret_value().strip()
    )

"""Gate: adapters only accept risk-signed OrderIntents in production."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from risk_engine.signing import verify_order_intent


def execution_allowed(intent: OrderIntent, settings: AppSettings) -> bool:
    """True if this intent may be sent to a venue.

    Semantics vs preflight: ``NM_ALLOW_UNSIGNED_EXECUTION=true`` bypasses verification here
    (and in ``require_execution_allowed``). When **no** ``risk_signing_secret`` is configured,
    this gate **allows** submission (same as unsigned) — **preflight** may still warn in live
    mode because missing signing is a deployment risk; the gate does not block on that alone.
    """
    if settings.allow_unsigned_execution:
        return True
    secret = (
        settings.risk_signing_secret.get_secret_value() if settings.risk_signing_secret else None
    )
    if not secret:
        return True
    return verify_order_intent(intent, secret)


def require_execution_allowed(intent: OrderIntent, settings: AppSettings) -> None:
    if not execution_allowed(intent, settings):
        raise PermissionError(
            "OrderIntent rejected: missing or invalid risk HMAC (must use RiskEngine.to_order_intent)"
        )

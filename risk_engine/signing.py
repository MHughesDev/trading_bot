"""HMAC signatures so only RiskEngine can produce submittable OrderIntents."""

from __future__ import annotations

import hashlib
import hmac
import json
from datetime import UTC, datetime

from app.contracts.orders import OrderIntent


_RISK_META_EXCLUDE = frozenset({"risk_signature", "risk_signed_at", "risk_key_id"})


def _canonical_payload(intent: OrderIntent) -> bytes:
    """Stable JSON for signing (excludes signature fields inside metadata)."""
    meta = {k: v for k, v in intent.metadata.items() if k not in _RISK_META_EXCLUDE}
    body = {
        "symbol": intent.symbol,
        "side": intent.side.value,
        "quantity": str(intent.quantity),
        "order_type": intent.order_type.value,
        "limit_price": str(intent.limit_price) if intent.limit_price is not None else None,
        "stop_price": str(intent.stop_price) if intent.stop_price is not None else None,
        "time_in_force": intent.time_in_force.value,
        "client_order_id": intent.client_order_id,
        "metadata": meta,
    }
    return json.dumps(body, sort_keys=True, separators=(",", ":")).encode("utf-8")


def sign_order_intent(intent: OrderIntent, secret: str, *, key_id: str = "v1") -> OrderIntent:
    """Return a copy of intent with metadata.risk_signature and risk_signed_at set."""
    base = intent.model_copy(deep=True)
    base.metadata = {k: v for k, v in base.metadata.items() if k not in _RISK_META_EXCLUDE}
    sig_body = _canonical_payload(base)
    digest = hmac.new(secret.encode("utf-8"), sig_body, hashlib.sha256).hexdigest()
    out = base.model_copy(deep=True)
    out.metadata["risk_signature"] = digest
    out.metadata["risk_signed_at"] = datetime.now(UTC).isoformat()
    out.metadata["risk_key_id"] = key_id
    return out


def verify_order_intent(intent: OrderIntent, secret: str) -> bool:
    """Constant-time verify; intent must include risk_signature in metadata."""
    sig = intent.metadata.get("risk_signature")
    if not sig or not isinstance(sig, str):
        return False
    stripped = intent.model_copy(deep=True)
    stripped.metadata = {k: v for k, v in stripped.metadata.items() if k not in _RISK_META_EXCLUDE}
    expected = hmac.new(secret.encode("utf-8"), _canonical_payload(stripped), hashlib.sha256).hexdigest()
    return hmac.compare_digest(sig, expected)

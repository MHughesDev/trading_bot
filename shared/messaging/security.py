"""Lightweight payload signing helpers for message-level risk/execution gating."""

from __future__ import annotations

import hashlib
import hmac
import json
from typing import Any


def sign_payload(payload: dict[str, Any], secret: str) -> str:
    """HMAC-SHA256 over sorted JSON payload."""
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hmac.new(secret.encode("utf-8"), raw, hashlib.sha256).hexdigest()


def verify_payload(payload: dict[str, Any], signature: str, secret: str) -> bool:
    expected = sign_payload(payload, secret)
    return hmac.compare_digest(signature, expected)

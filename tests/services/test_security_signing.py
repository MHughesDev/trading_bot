from __future__ import annotations

from shared.messaging.security import sign_payload, verify_payload


def test_sign_and_verify_payload() -> None:
    payload = {"symbol": "BTC/USD", "qty": 1.25, "side": "buy"}
    secret = "s3cr3t"
    sig = sign_payload(payload, secret)

    assert sig
    assert verify_payload(payload, sig, secret)
    assert not verify_payload(payload, sig, "wrong")

"""FB-UX-018: holdings sidebar helpers."""

from __future__ import annotations

from control_plane.streamlit_chrome import _position_rows_from_payload


def test_position_rows_ignores_invalid_and_zero_quantities() -> None:
    payload = {
        "positions": [
            {"symbol": "BTC-USD", "quantity": "bad"},
            {"symbol": "ETH-USD", "quantity": "0"},
            {"symbol": "SOL-USD", "quantity": "0.0001"},
        ]
    }
    out = _position_rows_from_payload(payload)
    assert len(out) == 1
    assert out[0]["symbol"] == "SOL-USD"


def test_position_rows_handles_non_dict_rows() -> None:
    payload = {"positions": [{"symbol": "BTC-USD", "quantity": "1"}, "oops", None]}
    out = _position_rows_from_payload(payload)
    assert len(out) == 1
    assert out[0]["symbol"] == "BTC-USD"

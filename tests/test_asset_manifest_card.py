"""FB-UX-010: manifest / model card row builder."""

from __future__ import annotations

from control_plane.asset_manifest_card import manifest_model_rows


def test_manifest_model_rows_empty_paths() -> None:
    m = {
        "canonical_symbol": "BTC-USD",
        "schema_version": "1",
        "forecaster_last_trained_at": None,
        "rl_last_trained_at": None,
        "forecaster_torch_path": None,
    }
    rows = manifest_model_rows(m)
    assert any(r["Field"] == "Last trained (forecaster)" for r in rows)
    ft = next(r for r in rows if r["Field"] == "Last trained (forecaster)")
    assert ft["Value"] == "—"


def test_manifest_model_rows_populated() -> None:
    m = {
        "forecaster_last_trained_at": "2026-01-01T00:00:00Z",
        "rl_last_trained_at": "2026-01-02T00:00:00Z",
        "forecaster_torch_path": "/data/run/forecaster.pt",
        "policy_mlp_path": "/data/run/policy.npz",
    }
    rows = manifest_model_rows(m)
    by_field = {r["Field"]: r["Value"] for r in rows}
    assert by_field["Last trained (forecaster)"] == "2026-01-01T00:00:00Z"
    assert by_field["Forecaster (torch)"] == "/data/run/forecaster.pt"
    assert by_field["Policy (MLP)"] == "/data/run/policy.npz"

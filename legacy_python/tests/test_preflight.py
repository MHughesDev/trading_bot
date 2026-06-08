"""Preflight report (IL-105 / FB-SPEC-08)."""

from __future__ import annotations

from pydantic import SecretStr

from app.config.settings import AppSettings
from control_plane.preflight import preflight_report


def test_live_requires_signing() -> None:
    s = AppSettings(
        execution_mode="live",
        risk_signing_secret=None,
        allow_unsigned_execution=False,
    )
    r = preflight_report(s)
    assert r["ok"] is False
    assert any("risk_signing" in x.lower() or "secret" in x.lower() for x in r["issues"])


def test_live_unsigned_flag_blocks() -> None:
    s = AppSettings(
        execution_mode="live",
        risk_signing_secret=SecretStr("x"),
        allow_unsigned_execution=True,
    )
    r = preflight_report(s)
    assert r["ok"] is False
    assert any("unsigned" in x.lower() for x in r["issues"])


def test_paper_ok_with_signing() -> None:
    s = AppSettings(
        execution_mode="paper",
        risk_signing_secret=SecretStr("secret"),
        allow_unsigned_execution=False,
        execution_paper_adapter="alpaca",
        alpaca_api_key=SecretStr("k"),
        alpaca_api_secret=SecretStr("s"),
    )
    r = preflight_report(s)
    assert r["ok"] is True


def test_bad_adapter_name() -> None:
    s = AppSettings(execution_mode="paper", execution_paper_adapter="wrong")
    r = preflight_report(s)
    assert r["ok"] is False

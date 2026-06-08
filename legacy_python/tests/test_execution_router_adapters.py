"""Execution router adapter selection (NM_EXECUTION_ADAPTER)."""

from __future__ import annotations

import pytest
from pydantic import SecretStr

from app.config.settings import AppSettings
from execution.adapters.mock_alpaca_paper import MockAlpacaPaperExecutionAdapter
from execution.adapters.stub import StubExecutionAdapter
from execution.router import create_execution_adapter


def test_router_selects_stub(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "stub")
    a = create_execution_adapter(AppSettings())
    assert isinstance(a, StubExecutionAdapter)


def test_router_selects_mock_alpaca_paper(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "mock_alpaca_paper")
    a = create_execution_adapter(
        AppSettings(
            risk_signing_secret=SecretStr("x" * 32),
            alpaca_api_key=SecretStr("k"),
            alpaca_api_secret=SecretStr("s"),
        )
    )
    assert isinstance(a, MockAlpacaPaperExecutionAdapter)


def test_router_default_paper_is_alpaca_when_no_nm_execution_adapter(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("NM_EXECUTION_ADAPTER", raising=False)
    pytest.importorskip("alpaca")
    a = create_execution_adapter(
        AppSettings(
            alpaca_api_key=SecretStr("k"),
            alpaca_api_secret=SecretStr("s"),
        )
    )
    assert a.name == "alpaca_paper"

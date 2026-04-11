"""Deployment checks for live/paper — complements IL-105 / FB-SPEC-08."""

from __future__ import annotations

from typing import Any, Literal

from app.config.settings import AppSettings


def preflight_report(settings: AppSettings) -> dict[str, Any]:
    """
    Non-secret readiness summary for operators and `GET /status`.

    Rules:
    - **live** execution: require signing secret; disallow unsigned execution.
    - **paper**: recommend Alpaca credentials when using default paper adapter.
    - **live** + Coinbase adapter: recommend Coinbase CDP credentials.
    """
    mode = settings.execution_mode
    issues: list[str] = []
    warnings: list[str] = []

    signing = bool(settings.risk_signing_secret and settings.risk_signing_secret.get_secret_value())
    if mode == "live":
        if not signing:
            issues.append("NM_RISK_SIGNING_SECRET is unset — live execution should use signed OrderIntents")
        if settings.allow_unsigned_execution:
            issues.append("NM_ALLOW_UNSIGNED_EXECUTION=true is unsafe for live execution")
    else:
        if not signing:
            warnings.append("NM_RISK_SIGNING_SECRET unset — acceptable for local paper; set for production-like paper")

    if mode == "paper" and settings.execution_paper_adapter.lower() == "alpaca":
        if not settings.alpaca_api_key or not settings.alpaca_api_secret:
            warnings.append("NM_ALPACA_API_KEY / NM_ALPACA_API_SECRET unset — paper orders may fail")

    if mode == "live" and settings.execution_live_adapter.lower() == "coinbase":
        if not settings.coinbase_api_key or not settings.coinbase_api_secret:
            warnings.append("NM_COINBASE_API_KEY / NM_COINBASE_API_SECRET unset — live Coinbase adapter may fail")

    if mode == "paper":
        _validate_adapter(settings.execution_paper_adapter, ("alpaca",), "PAPER", issues)
    if mode == "live":
        _validate_adapter(settings.execution_live_adapter, ("coinbase",), "LIVE", issues)

    ok = len(issues) == 0
    severity: Literal["ok", "warn", "block"] = "block" if not ok else ("warn" if warnings else "ok")
    return {
        "ok": ok,
        "severity": severity,
        "execution_mode": mode,
        "issues": issues,
        "warnings": warnings,
    }


def _validate_adapter(name: str, allowed: tuple[str, ...], env_hint: str, issues: list[str]) -> None:
    n = (name or "").lower().strip()
    if n and n not in allowed:
        issues.append(f"execution_{env_hint.lower()}_adapter={name!r} not in {allowed} (check NM_EXECUTION_{env_hint}_ADAPTER)")


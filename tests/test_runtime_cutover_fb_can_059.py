"""FB-CAN-059: runtime cutover guard (bridge + in_process without migration shadow)."""

from __future__ import annotations

import pytest

from app.config.canonical_config import merge_canonical, synthesize_canonical_from_legacy
from app.config.runtime_cutover import validate_runtime_cutover
from app.config.settings import AppSettings, load_settings


def test_validate_raises_when_bridge_in_process_without_migration_shadow() -> None:
    s = AppSettings(
        microservices_runtime_bridge_enabled=True,
        microservices_execution_gateway_mode="in_process",
    )
    base = synthesize_canonical_from_legacy(s)
    assert base.domains.runtime_cutover.get("migration_shadow_allowed") is False
    with pytest.raises(RuntimeError, match="Runtime cutover guard"):
        validate_runtime_cutover(s)


def test_validate_ok_when_migration_shadow_allowed() -> None:
    s = AppSettings(
        microservices_runtime_bridge_enabled=True,
        microservices_execution_gateway_mode="in_process",
    )
    base = synthesize_canonical_from_legacy(s)
    rc = dict(base.domains.runtime_cutover)
    rc["migration_shadow_allowed"] = True
    override = base.model_copy(
        update={"domains": base.domains.model_copy(update={"runtime_cutover": rc})},
    )
    merged = merge_canonical(base, override)
    s._canonical_runtime = merged  # noqa: SLF001
    validate_runtime_cutover(s)


def test_validate_ok_when_bridge_disabled() -> None:
    s = AppSettings(
        microservices_runtime_bridge_enabled=False,
        microservices_execution_gateway_mode="in_process",
    )
    validate_runtime_cutover(s)


def test_validate_ok_when_gateway_external() -> None:
    s = AppSettings(
        microservices_runtime_bridge_enabled=True,
        microservices_execution_gateway_mode="external",
    )
    validate_runtime_cutover(s)


def test_load_settings_passes_with_repo_defaults() -> None:
    load_settings()

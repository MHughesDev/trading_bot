"""Tests for APEX canonical configuration (FB-CAN-003)."""

from __future__ import annotations

from pathlib import Path

import yaml

from app.config.canonical_config import (
    merge_canonical,
    parse_canonical_from_yaml_fragment,
    resolve_canonical_config,
    synthesize_canonical_from_app_settings,
)
from app.config.canonical_metadata_validation import validate_canonical_metadata_complete
from app.config.settings import AppSettings, load_settings


def test_synthesize_contains_risk_and_replay_domains():
    s = AppSettings()
    c = synthesize_canonical_from_app_settings(s)
    validate_canonical_metadata_complete(c.metadata)
    assert c.metadata.config_version == "1.0.0"
    assert c.metadata.config_name == "app-settings-synthesis"
    assert c.metadata.created_by == "app-settings-synthesis"
    assert c.domains.risk_sizing.get("source") == "app_settings"
    assert c.domains.risk_sizing["max_total_exposure_usd"] == s.risk_max_total_exposure_usd
    assert c.domains.replay["backtesting_slippage_bps"] == s.backtesting_slippage_bps


def test_resolve_merges_yaml_metadata():
    s = AppSettings()
    raw = {
        "apex_canonical": {
            "metadata": {
                "config_name": "overlay-test",
                "config_version": "1.1.0",
                "notes": "from unit test",
            }
        }
    }
    c = resolve_canonical_config(s, raw)
    assert c.metadata.config_name == "overlay-test"
    assert c.metadata.config_version == "1.1.0"
    assert c.domains.risk_sizing["max_total_exposure_usd"] == s.risk_max_total_exposure_usd


def test_merge_canonical_deep_merges_domains():
    from app.config.canonical_config import CanonicalDomains, CanonicalMetadata, CanonicalRuntimeConfig

    fam = ["market_microstructure"]
    a = CanonicalRuntimeConfig(
        metadata=CanonicalMetadata(
            config_version="1.0.0",
            config_name="a",
            notes="base",
            enabled_feature_families=fam,
        ),
        domains=CanonicalDomains(risk_sizing={"x": 1, "nested": {"y": 2}}),
    )
    b = CanonicalRuntimeConfig(
        metadata=CanonicalMetadata(
            config_version="1.1.0",
            config_name="b",
            notes="overlay",
            enabled_feature_families=fam,
        ),
        domains=CanonicalDomains(risk_sizing={"nested": {"z": 3}}),
    )
    m = merge_canonical(a, b)
    assert m.metadata.config_version == "1.1.0"
    assert m.domains.risk_sizing["x"] == 1
    assert m.domains.risk_sizing["nested"]["y"] == 2
    assert m.domains.risk_sizing["nested"]["z"] == 3


def test_load_settings_binds_canonical(tmp_path: Path, monkeypatch):
    yml = tmp_path / "cfg.yaml"
    yml.write_text(
        yaml.safe_dump(
            {
                "apex_canonical": {
                    "metadata": {"config_name": "tmp-file", "config_version": "2.0.0"},
                    "domains": {
                        "trigger": {"stage_timeout_ms": 500},
                        "risk_sizing": {"max_total_exposure_usd": 99999},
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    import app.config.settings as settings_mod

    monkeypatch.setattr(settings_mod, "_DEFAULT_YAML", yml)
    s = load_settings()
    assert s.risk_max_total_exposure_usd == 99999
    c = s.canonical
    assert c.metadata.config_name == "tmp-file"
    assert c.domains.trigger["stage_timeout_ms"] == 500
    assert c.domains.risk_sizing["max_total_exposure_usd"] == 99999


def test_parse_canonical_from_yaml_fragment():
    raw = {
        "metadata": {
            "config_version": "3.0.0",
            "config_name": "p",
            "created_at": "2026-01-01T00:00:00+00:00",
            "created_by": "test",
            "notes": "unit test fragment",
            "enabled_feature_families": ["market_microstructure"],
        },
        "domains": {"auction": {"foo": "bar"}},
    }
    c = parse_canonical_from_yaml_fragment(raw)
    assert c is not None
    assert c.metadata.config_version == "3.0.0"
    assert c.domains.auction["foo"] == "bar"

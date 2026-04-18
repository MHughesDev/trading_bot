"""FB-CAN-057: canonical config diff audit and semantics."""

from __future__ import annotations

from pathlib import Path

import yaml

from app.config.settings import AppSettings
from app.config.canonical_config import resolve_canonical_config
from orchestration.config_diff_audit import (
    append_config_diff_audit_entry,
    build_canonical_config_diff_report,
    read_config_diff_audit_tail,
)


def test_semantic_flags_risk_change(tmp_path: Path) -> None:
    settings = AppSettings()
    raw_b = yaml.safe_load(
        """
apex_canonical:
  metadata: {config_version: "a", config_name: "t"}
  domains:
    risk_sizing: {max_total_exposure_usd: 100.0}
"""
    )
    raw_c = yaml.safe_load(
        """
apex_canonical:
  metadata: {config_version: "b", config_name: "t"}
  domains:
    risk_sizing: {max_total_exposure_usd: 200.0}
"""
    )
    b = resolve_canonical_config(settings, raw_b)
    c = resolve_canonical_config(settings, raw_c)
    rep = build_canonical_config_diff_report(b, c)
    assert rep["semantic_analysis"]["requires_operator_review"] is True
    assert "risk_sizing" in rep["semantic_analysis"]["semantic_categories"]


def test_audit_append_and_read(tmp_path: Path) -> None:
    settings = AppSettings()
    raw_b = {"apex_canonical": {"metadata": {"config_version": "1", "config_name": "x"}}}
    raw_c = {"apex_canonical": {"metadata": {"config_version": "2", "config_name": "x"}}}
    b = resolve_canonical_config(settings, raw_b)
    c = resolve_canonical_config(settings, raw_c)
    rep = build_canonical_config_diff_report(b, c)
    p = tmp_path / "audit.jsonl"
    append_config_diff_audit_entry(rep, path=p)
    append_config_diff_audit_entry(rep, path=p)
    tail = read_config_diff_audit_tail(path=p, limit=10)
    assert len(tail) == 2

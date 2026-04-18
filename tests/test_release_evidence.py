"""FB-CAN-026: release evidence bundles and canonical diffs."""

from __future__ import annotations

from pathlib import Path

import yaml

from app.config.settings import AppSettings
from orchestration.release_evidence import (
    build_release_evidence_bundle,
    canonical_runtime_fingerprint,
    diff_canonical_runtime,
    resolve_canonical_from_yaml_file,
)
from orchestration.release_gating import RollbackTarget


def test_fingerprint_stable_for_same_config():
    s = AppSettings()
    a = s.canonical
    b = s.canonical
    assert canonical_runtime_fingerprint(a) == canonical_runtime_fingerprint(b)


def test_diff_empty_when_same(tmp_path: Path) -> None:
    p = tmp_path / "cfg.yaml"
    p.write_text(
        yaml.dump(
            {
                "apex_canonical": {
                    "metadata": {
                        "config_version": "1.0.0",
                        "config_name": "t",
                        "environment_scope": "research",
                    },
                    "domains": {"risk_sizing": {"max_total_exposure_usd": 100.0}},
                }
            }
        ),
        encoding="utf-8",
    )
    left = resolve_canonical_from_yaml_file(p)
    right = resolve_canonical_from_yaml_file(p)
    d = diff_canonical_runtime(left, right)
    assert d["change_count"] == 0


def test_bundle_includes_fingerprint():
    b = build_release_evidence_bundle(
        rollback=RollbackTarget(target_config_version="0.9.0"),
    )
    assert len(b.canonical_config_fingerprint) == 64
    assert b.config_version


def test_bundle_with_baseline_diff(tmp_path: Path) -> None:
    base = tmp_path / "base.yaml"
    base.write_text(
        yaml.dump(
            {
                "apex_canonical": {
                    "metadata": {
                        "config_version": "0.9.0",
                        "config_name": "old",
                        "environment_scope": "research",
                    },
                    "domains": {"risk_sizing": {"max_total_exposure_usd": 1.0}},
                }
            }
        ),
        encoding="utf-8",
    )
    b = build_release_evidence_bundle(baseline_yaml_path=base)
    assert b.canonical_diff_vs_baseline is not None
    assert b.canonical_diff_vs_baseline["change_count"] >= 1

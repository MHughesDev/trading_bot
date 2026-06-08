#!/usr/bin/env python3
"""FB-CAN-025 / FB-CAN-061: enforce APEX canonical config surface on default.yaml + loaded settings.

- ``apex_canonical.metadata`` must include required §4 fields (version, name, created_at, created_by,
  notes, enabled_feature_families).
- ``apex_canonical.domains.risk_sizing`` must exist (FB-CAN-022 cutover).
- :func:`load_settings` must produce a valid :class:`CanonicalRuntimeConfig` bundle.
"""
from __future__ import annotations

import sys
from pathlib import Path

import yaml

from app.config.post_release_probation import validate_post_release_probation_domain
from app.config.shadow_comparison import validate_shadow_comparison_domain
from app.config.signal_confidence import validate_signal_confidence_domain

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    default_yaml = ROOT / "app" / "config" / "default.yaml"
    raw = yaml.safe_load(default_yaml.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        print("ci_canonical_config_gates: default.yaml root must be a mapping", file=sys.stderr)
        return 1

    ac = raw.get("apex_canonical")
    if not isinstance(ac, dict):
        print("ci_canonical_config_gates: missing apex_canonical block in default.yaml", file=sys.stderr)
        return 1

    meta = ac.get("metadata")
    if not isinstance(meta, dict):
        print("ci_canonical_config_gates: apex_canonical.metadata must be a mapping", file=sys.stderr)
        return 1

    cv = meta.get("config_version")
    cn = meta.get("config_name")
    if not cv or not str(cv).strip():
        print("ci_canonical_config_gates: apex_canonical.metadata.config_version is required", file=sys.stderr)
        return 1
    if not cn or not str(cn).strip():
        print("ci_canonical_config_gates: apex_canonical.metadata.config_name is required", file=sys.stderr)
        return 1
    ca = meta.get("created_at")
    if not ca or not str(ca).strip():
        print("ci_canonical_config_gates: apex_canonical.metadata.created_at is required (FB-CAN-061)", file=sys.stderr)
        return 1
    cb = meta.get("created_by")
    if not cb or not str(cb).strip():
        print("ci_canonical_config_gates: apex_canonical.metadata.created_by is required (FB-CAN-061)", file=sys.stderr)
        return 1
    nt = meta.get("notes")
    if not nt or not str(nt).strip():
        print("ci_canonical_config_gates: apex_canonical.metadata.notes is required (FB-CAN-061)", file=sys.stderr)
        return 1
    eff = meta.get("enabled_feature_families")
    if not isinstance(eff, list) or len(eff) == 0:
        print(
            "ci_canonical_config_gates: apex_canonical.metadata.enabled_feature_families must be non-empty (FB-CAN-061)",
            file=sys.stderr,
        )
        return 1

    domains = ac.get("domains")
    if not isinstance(domains, dict):
        print("ci_canonical_config_gates: apex_canonical.domains must be a mapping", file=sys.stderr)
        return 1
    if "risk_sizing" not in domains:
        print(
            "ci_canonical_config_gates: apex_canonical.domains.risk_sizing is required (FB-CAN-022)",
            file=sys.stderr,
        )
        return 1
    sc = domains.get("signal_confidence")
    if not isinstance(sc, dict):
        print(
            "ci_canonical_config_gates: apex_canonical.domains.signal_confidence is required (FB-CAN-032)",
            file=sys.stderr,
        )
        return 1
    ff = domains.get("feature_families")
    if not isinstance(ff, dict):
        print(
            "ci_canonical_config_gates: apex_canonical.domains.feature_families is required (FB-CAN-032)",
            file=sys.stderr,
        )
        return 1
    for err in validate_signal_confidence_domain(sc):
        print(f"ci_canonical_config_gates: {err}", file=sys.stderr)
        return 1

    for err in validate_shadow_comparison_domain(domains.get("shadow_comparison")):
        print(f"ci_canonical_config_gates: {err}", file=sys.stderr)
        return 1

    for err in validate_post_release_probation_domain(domains.get("post_release_probation")):
        print(f"ci_canonical_config_gates: {err}", file=sys.stderr)
        return 1

    from app.config.settings import load_settings

    settings = load_settings()
    cr = settings.canonical
    if not str(cr.metadata.config_version).strip():
        print("ci_canonical_config_gates: resolved canonical.metadata.config_version is empty", file=sys.stderr)
        return 1
    if not str(cr.metadata.config_name).strip():
        print("ci_canonical_config_gates: resolved canonical.metadata.config_name is empty", file=sys.stderr)
        return 1

    print("ci_canonical_config_gates: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

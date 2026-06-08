#!/usr/bin/env bash
# FB-CAN-053 — rollback playbook fields validated on the same fixture as release gates.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
from pathlib import Path

from app.config.settings import load_settings
from app.config.canonical_config import resolve_canonical_config
import yaml

from app.contracts.release_objects import ReleaseCandidate
from orchestration.rollback_validation import validate_rollback_playbook
from orchestration.release_evidence import resolve_canonical_from_yaml_text

fixture = Path("tests/fixtures/canonical_release_candidate_live.json")
cand = ReleaseCandidate.model_validate(
    __import__("json").loads(fixture.read_text(encoding="utf-8"))
)
ok, reasons = validate_rollback_playbook(cand.rollback, target_environment="live")
if not ok:
    raise SystemExit("rollback playbook validation failed: " + "; ".join(reasons))

# Viability: merged canonical at rollback target version string loads (config releases).
settings = load_settings()
raw = yaml.safe_load(Path("app/config/default.yaml").read_text(encoding="utf-8"))
if not isinstance(raw, dict):
    raise SystemExit("default.yaml root must be mapping")
rb_ver = (cand.rollback.target_config_version or "").strip()
if rb_ver:
    meta = dict(raw.get("apex_canonical") or {}).get("metadata") or {}
    meta = dict(meta)
    meta["config_version"] = rb_ver
    apex = dict(raw.get("apex_canonical") or {})
    apex["metadata"] = meta
    raw2 = dict(raw)
    raw2["apex_canonical"] = apex
    resolve_canonical_config(settings, raw2)

print("ci_rollback_playbook: OK")
PY

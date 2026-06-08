#!/usr/bin/env bash
# FB-CAN-078 — canonical CI gates without the acceptance audit (avoids recursion).
# Invoked by ``ci_canonical_acceptance_audit.py`` and by ``ci_canonical_gates.sh`` (before the audit).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 scripts/ci_canonical_config_gates.py
bash scripts/ci_forbidden_legacy_paths.sh
python3 scripts/ci_replay_determinism.py
python3 scripts/ci_live_replay_equivalence.py
python3 scripts/ci_monitoring_domain_checklist.py
python3 scripts/ci_config_diff_semantics.py
python3 scripts/ci_no_migration_projection_legacy.py
python3 scripts/ci_canonical_magic_constants.py
python3 scripts/ci_canonical_coverage_matrix.py

echo "ci_canonical_gates_inner: OK"

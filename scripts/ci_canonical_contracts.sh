#!/usr/bin/env bash
# FB-CAN-013 — merge gate: release-gating CLI accepts the checked-in live candidate fixture.
# FB-CAN-015 — typed snapshot models (`tests/test_decision_snapshots.py`) run in main pytest.
# FB-CAN-025 — broader canonical gates live in `ci_canonical_gates.sh` (also run in CI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 scripts/validate_release_gates.py \
  --candidate tests/fixtures/canonical_release_candidate_live.json \
  --target live

bash scripts/ci_rollback_playbook.sh

echo "ci_canonical_contracts: OK"

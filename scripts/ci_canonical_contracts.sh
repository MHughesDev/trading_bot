#!/usr/bin/env bash
# FB-CAN-013 — merge gate: release-gating CLI accepts the checked-in live candidate fixture.
# Canonical domain tests run in the main `pytest tests/` job (`test_canonical_*.py`, etc.).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 scripts/validate_release_gates.py \
  --candidate tests/fixtures/canonical_release_candidate_live.json \
  --target live

echo "ci_canonical_contracts: OK"

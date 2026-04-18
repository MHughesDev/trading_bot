#!/usr/bin/env bash
# FB-CAN-025 — aggregate canonical CI gates (config YAML, forbidden paths, replay determinism).
# FB-CAN-078 — runs inner gates then canonical acceptance audit (writes JSON report).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

bash scripts/ci_canonical_gates_inner.sh
python3 scripts/ci_canonical_acceptance_audit.py

echo "ci_canonical_gates: OK"

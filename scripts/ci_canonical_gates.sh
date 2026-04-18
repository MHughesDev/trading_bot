#!/usr/bin/env bash
# FB-CAN-025 — aggregate canonical CI gates (config YAML, forbidden paths, replay determinism).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 scripts/ci_canonical_config_gates.py
bash scripts/ci_forbidden_legacy_paths.sh
python3 scripts/ci_replay_determinism.py

echo "ci_canonical_gates: OK"

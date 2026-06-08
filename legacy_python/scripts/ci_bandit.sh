#!/usr/bin/env bash
# Python SAST — High severity only (FB-AUD-020). Medium/Low reviewed separately.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
exec python3 -m bandit -c pyproject.toml -r \
  app control_plane execution data_plane decision_engine risk_engine orchestration \
  observability forecaster_model policy_model services \
  --exclude '*/tests/*' -lll -q

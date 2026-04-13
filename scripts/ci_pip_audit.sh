#!/usr/bin/env bash
# Audits only dependencies installed into a dedicated venv (`.audit-venv/`), not system/apt Python.
# This keeps CI green while still failing on vulnerable **project** transitive deps (FB-AUD-018).
# Requires `python3 -m venv` (ensurepip) — true on GitHub-hosted runners.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VENV="${AUDIT_VENV_PATH:-$ROOT/.audit-venv}"
if [[ ! -x "$VENV/bin/python" ]]; then
  python3 -m venv "$VENV"
fi
"$VENV/bin/python" -m pip install -q -U pip setuptools wheel
"$VENV/bin/pip" install -q -e ".[dev]"
exec "$VENV/bin/python" -m pip_audit -l "$@"

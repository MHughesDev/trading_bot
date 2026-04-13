#!/usr/bin/env bash
# Audits only dependencies installed into a dedicated venv (`.audit-venv/`), not system/apt Python.
# This keeps CI green while still failing on vulnerable **project** transitive deps (FB-AUD-018).
# If project `.venv` already has `pip_audit`, reuse it to avoid duplicate bootstrap work.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [[ -x "$ROOT/.venv/bin/python" ]] && "$ROOT/.venv/bin/python" -c "import pip_audit" >/dev/null 2>&1; then
  exec "$ROOT/.venv/bin/python" -m pip_audit -l "$@"
fi
VENV="${AUDIT_VENV_PATH:-$ROOT/.audit-venv}"
if [[ ! -x "$VENV/bin/python" ]]; then
  python3 -m venv "$VENV"
fi
"$VENV/bin/python" scripts/env_preflight.py || {
  echo "ci_pip_audit: package-index preflight failed." >&2
  exit 1
}
"$VENV/bin/python" -m pip install -q -U pip setuptools wheel || {
  echo "ci_pip_audit: failed to bootstrap audit venv." >&2
  echo "Hint: verify proxy/index settings (HTTP(S)_PROXY, PIP_INDEX_URL) or pre-install deps in .venv." >&2
  exit 1
}
"$VENV/bin/pip" install -q -e ".[dev]" || {
  echo "ci_pip_audit: failed to install project dev dependencies for audit." >&2
  echo "Hint: run './setup.sh' (or setup.bat) and ensure package index access is available." >&2
  exit 1
}
exec "$VENV/bin/python" -m pip_audit -l "$@"

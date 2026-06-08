#!/usr/bin/env bash
# Trading Bot environment doctor (Linux/macOS)
set -euo pipefail
cd "$(dirname "$0")"

step() { printf "\n[doctor] %s\n" "$1"; }

step "Python check"
python3 --version
python3 - <<'PY'
import sys
assert (sys.version_info.major, sys.version_info.minor) >= (3, 11), "Python >=3.11 required"
print("python_ok:", sys.version.split()[0])
PY

step "Proxy/index vars"
echo "HTTP_PROXY=${HTTP_PROXY:-<unset>}"
echo "HTTPS_PROXY=${HTTPS_PROXY:-<unset>}"
echo "PIP_INDEX_URL=${PIP_INDEX_URL:-<unset>}"
echo "PIP_EXTRA_INDEX_URL=${PIP_EXTRA_INDEX_URL:-<unset>}"

step "Package index preflight"
python3 scripts/env_preflight.py

step "Create virtualenv + install dev dependencies"
python3 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip setuptools wheel
pip install -e ".[dev]"

step "Module import smoke"
python - <<'PY'
mods = ["pydantic", "fastapi", "httpx", "yaml", "polars", "numpy", "pytest", "ruff", "pip_audit", "bandit"]
missing = []
for m in mods:
    try:
        __import__(m)
    except Exception:
        missing.append(m)
if missing:
    raise SystemExit(f"Missing modules: {missing}")
print("all required modules import OK")
PY

step "Full local checks"
python -m ruff check .
python -m pytest tests/ -q
bash scripts/ci_spec_compliance.sh
python3 scripts/ci_queue_consistency.py
bash scripts/ci_pip_audit.sh
bash scripts/ci_bandit.sh
bash scripts/ci_mlflow_promotion_policy.sh

echo "\nSUCCESS: environment is audit-ready."

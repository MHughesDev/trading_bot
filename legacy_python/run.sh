#!/usr/bin/env bash
# Trading Bot — run API + supervisor (Linux / macOS). Parity with run.bat.
# The React frontend is built into frontend/dist/ and served by FastAPI.
# Usage: from repo root after ./setup.sh:  chmod +x run.sh && ./run.sh
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(pwd)"
VPY="${ROOT}/.venv/bin/python"


print_banner() {
  echo "🚀============================================🚀"
  echo "  Trading Bot Launchpad"
  echo "🚀============================================🚀"
}

fun_step() {
  local label="$1"
  printf "%s" "$label"
  printf " ...\n"
}

require_alive() {
  local pid="$1"
  local name="$2"
  sleep 1
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "ERROR: ${name} exited during startup. Check logs above for missing dependencies/config."
    exit 1
  fi
}

print_banner

if [[ ! -x "$VPY" ]]; then
  echo "ERROR: .venv not found. Run ./setup.sh first."
  exit 1
fi

export PATH="${ROOT}/.venv/bin:${PATH}"
export NM_CONTROL_PLANE_URL="http://127.0.0.1:8001"
export NM_AUTH_SESSION_ENABLED="true"

cleanup() {
  echo ""
  echo "Stopping child processes..."
  [[ -n "${API_PID:-}" ]] && kill "$API_PID" 2>/dev/null || true
  [[ -n "${SUP_PID:-}" ]] && kill "$SUP_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Build React frontend if dist is missing
if [[ ! -f "${ROOT}/frontend/dist/index.html" ]]; then
  fun_step "Building React frontend"
  (cd "${ROOT}/frontend" && npm run build)
fi

fun_step "Spinning up Control Plane API + React UI"
"$VPY" -m uvicorn control_plane.api:app --host 127.0.0.1 --port 8001 &
API_PID=$!
require_alive "$API_PID" "Control Plane API"

sleep 1
fun_step "Starting Power Supervisor"
"$VPY" -m app.runtime.power_supervisor &
SUP_PID=$!
require_alive "$SUP_PID" "Power Supervisor"

echo ""
echo "Started (foreground — Ctrl+C stops all):"
echo "  - Dashboard + API: http://127.0.0.1:8001"
echo "  - Supervisor:      live runtime on 8208 when power ON"
echo ""
echo "To disable auto live runtime: export NM_POWER_SUPERVISOR_ENABLED=false before ./run.sh"
wait

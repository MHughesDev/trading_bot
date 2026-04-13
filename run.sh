#!/usr/bin/env bash
# Trading Bot — run API + supervisor + Streamlit (Linux / macOS). Parity with run.bat.
# Usage: from repo root after ./setup.sh:  chmod +x run.sh && ./run.sh
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(pwd)"
VPY="${ROOT}/.venv/bin/python"


print_banner() {
  echo "🚀==============================================🚀"
  echo "  Trading Bot Launchpad"
  echo "🚀==============================================🚀"
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

cleanup() {
  echo ""
  echo "Stopping child processes..."
  [[ -n "${API_PID:-}" ]] && kill "$API_PID" 2>/dev/null || true
  [[ -n "${SUP_PID:-}" ]] && kill "$SUP_PID" 2>/dev/null || true
  [[ -n "${UI_PID:-}" ]] && kill "$UI_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

fun_step "Spinning up Control Plane API"
"$VPY" -m uvicorn control_plane.api:app --host 127.0.0.1 --port 8000 &
API_PID=$!
require_alive "$API_PID" "Control Plane API"
sleep 1
fun_step "Starting Power Supervisor"
"$VPY" -m app.runtime.power_supervisor &
SUP_PID=$!
require_alive "$SUP_PID" "Power Supervisor"
fun_step "Launching Streamlit dashboard"
"$VPY" -m streamlit run control_plane/Home.py --server.headless true &
UI_PID=$!
require_alive "$UI_PID" "Streamlit dashboard"

echo ""
echo "Started (foreground — Ctrl+C stops all):"
echo "  - Control plane: http://127.0.0.1:8000"
echo "  - Supervisor:    live runtime on 8208 when power ON (sidebar)"
echo "  - Dashboard:     Streamlit prints URL (usually http://localhost:8501)"
echo ""
echo "To disable auto live runtime: export NM_POWER_SUPERVISOR_ENABLED=false before ./run.sh"
wait

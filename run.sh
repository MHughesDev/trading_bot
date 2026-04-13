#!/usr/bin/env bash
# Trading Bot — run API + supervisor + Streamlit (Linux / macOS). Parity with run.bat.
# Usage: from repo root after ./setup.sh:  chmod +x run.sh && ./run.sh
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(pwd)"
VPY="${ROOT}/.venv/bin/python"

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

"$VPY" -m uvicorn control_plane.api:app --host 127.0.0.1 --port 8000 &
API_PID=$!
sleep 2
"$VPY" -m app.runtime.power_supervisor &
SUP_PID=$!
"$VPY" -m streamlit run control_plane/Home.py --server.headless true &
UI_PID=$!

echo ""
echo "Started (foreground — Ctrl+C stops all):"
echo "  - Control plane: http://127.0.0.1:8000"
echo "  - Supervisor:    live runtime on 8208 when power ON (sidebar)"
echo "  - Dashboard:     Streamlit prints URL (usually http://localhost:8501)"
echo ""
echo "To disable auto live runtime: export NM_POWER_SUPERVISOR_ENABLED=false before ./run.sh"
wait

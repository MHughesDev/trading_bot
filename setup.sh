#!/usr/bin/env bash
# Trading Bot — setup (Linux / macOS). Parity with setup.bat: venv, pip, Docker pull+up.
# Usage: from repo root:  chmod +x setup.sh && ./setup.sh
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(pwd)"


print_banner() {
  echo "✨==============================================✨"
  echo "  Trading Bot Setup Wizard"
  echo "✨==============================================✨"
}

fun_wait() {
  local label="$1"
  local seconds="${2:-2}"
  local i
  printf "%s" "$label"
  for ((i=0; i<seconds; i++)); do
    printf " ."
    sleep 1
  done
  printf " ✅\n"
}

print_banner
echo "=== Trading Bot — setup ==="
echo "Repo: $ROOT"
echo ""

if [[ "${NM_SKIP_DOCKER:-}" == "1" ]]; then
  echo "NM_SKIP_DOCKER=1 — skipping Docker steps."
fi

# --- Python 3.12 preferred (CI baseline), 3.11+ supported ---
PY=""
if command -v python3.12 &>/dev/null; then PY="python3.12"
elif command -v python3.11 &>/dev/null; then PY="python3.11"
elif command -v python3 &>/dev/null; then
  ver="$(python3 -c 'import sys; print("%d.%d"%sys.version_info[:2])' 2>/dev/null || echo 0.0)"
  major="${ver%%.*}"
  minor="${ver#*.}"
  if [[ "${major:-0}" -eq 3 && "${minor:-0}" -ge 11 ]]; then PY="python3"
  fi
fi
if [[ -z "$PY" ]]; then
  echo "ERROR: Python 3.11+ not found. Install python3.11 (e.g. apt install python3.11 python3.11-venv) and re-run."
  exit 1
fi
echo "Using: $PY"
$PY --version
selected_ver="$($PY -c 'import sys; print(f"{sys.version_info[0]}.{sys.version_info[1]}")')"
if [[ "$selected_ver" != "3.12" ]]; then
  echo "NOTE: CI uses Python 3.12. You selected $selected_ver (supported), which may differ from CI behavior."
fi

# --- venv ---
if [[ ! -x ".venv/bin/python" ]]; then
  echo "Creating virtual environment .venv ..."
  "$PY" -m venv .venv
else
  echo "Virtual environment .venv already exists."
fi

VPY="${ROOT}/.venv/bin/python"
fun_wait "Warming up package checks" 2
echo "Running package-index preflight ..."
if ! "$VPY" scripts/env_preflight.py; then
  echo "ERROR: package index is unreachable from this container. Resolve proxy/index settings, then retry."
  exit 1
fi

"$VPY" -m pip install --upgrade pip wheel setuptools

echo "Installing package with dev + dashboard (Streamlit) ..."
if ! "$VPY" -m pip install -e ".[dev,dashboard]"; then
  echo ""
  echo "ERROR: dependency install failed."
  echo "If you are behind a proxy, verify HTTP(S)_PROXY and set PIP_INDEX_URL to a reachable package index."
  echo "Then re-run ./setup.sh."
  exit 1
fi

# --- Docker ---
docker_compose() {
  if docker compose version &>/dev/null; then
    docker compose -f "${ROOT}/infra/docker-compose.yml" "$@"
  elif command -v docker-compose &>/dev/null; then
    docker-compose -f "${ROOT}/infra/docker-compose.yml" "$@"
  else
    echo "ERROR: Neither 'docker compose' nor 'docker-compose' found."
    return 1
  fi
}

if [[ "${NM_SKIP_DOCKER:-}" != "1" ]]; then
  if ! command -v docker &>/dev/null; then
    echo ""
    echo "Docker not found. Install the Docker Engine + Compose plugin, then re-run setup.sh."
    echo "  Debian/Ubuntu (example):"
    echo "    sudo apt-get update && sudo apt-get install -y ca-certificates curl"
    echo "    sudo install -m 0755 -d /etc/apt/keyrings"
    echo "    # See https://docs.docker.com/engine/install/ for your distro"
    echo "  Or: https://docs.docker.com/desktop/install/linux-install/"
    echo ""
    read -r -p "Continue without Docker (skip infra stack)? [y/N]: " ans || true
    if [[ "${ans:-}" =~ ^[yY]$ ]]; then
      echo "Skipping docker compose."
    else
      exit 1
    fi
  else
    echo "Waiting for Docker Engine (start: sudo systemctl start docker  OR  log in to Docker Desktop) ..."
    tries=0
    while ! docker info &>/dev/null; do
      tries=$((tries + 1))
      if [[ "$tries" -ge 60 ]]; then
        echo "WARNING: Docker not ready after ~3 minutes. Skipping compose. Fix Docker and re-run setup.sh"
        break
      fi
      sleep 3
    done
    if docker info &>/dev/null; then
      fun_wait "Docking with Docker Engine" 2
      echo "Pulling infra images (new tags after git pull) ..."
      docker_compose pull || echo "WARNING: docker compose pull failed — check network or docker login."
      echo "Starting infra stack (docker compose up -d) ..."
      docker_compose up -d || echo "WARNING: docker compose up failed."
    fi
  fi
fi

# --- .env ---
if [[ ! -f ".env" ]]; then
  if [[ -f ".env.example" ]]; then
    echo "Copying .env.example to .env — edit .env with your NM_* secrets."
    cp -f .env.example .env
  else
    echo "NOTE: Create a .env file in the repo root (see README / .env.example)."
  fi
else
  echo ".env already present."
fi

echo ""
echo "=== Setup finished ==="
echo "Next: edit .env (NM_ALPACA_* for paper, NM_RISK_SIGNING_SECRET, etc.)"
echo "Then run: ./run.sh"
exit 0

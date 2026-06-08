#!/usr/bin/env bash
# FB-CONT-001 — one process per container; use Compose for multiple services.
set -euo pipefail

cmd="${1:-api}"
shift || true

case "$cmd" in
  api)
    # Serves the REST API + React SPA (frontend/dist/) on port 8001.
    exec uvicorn control_plane.api:app --host 0.0.0.0 --port 8001 "$@"
    ;;
  live)
    exec python3 -m app.runtime.live_service "$@"
    ;;
  shell)
    exec bash "$@"
    ;;
  *)
    exec "$cmd" "$@"
    ;;
esac

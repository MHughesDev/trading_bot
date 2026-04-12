#!/usr/bin/env bash
# FB-CONT-001 — one process per container; use Compose for multiple services.
set -euo pipefail

cmd="${1:-api}"
shift || true

case "$cmd" in
  api)
    exec uvicorn control_plane.api:app --host 0.0.0.0 --port 8000 "$@"
    ;;
  streamlit)
    exec python3 -m streamlit run control_plane/Home.py \
      --server.address 0.0.0.0 \
      --server.port 8501 \
      "$@"
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

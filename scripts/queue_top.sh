#!/usr/bin/env bash
# Print the next Open queue row (repo root). Agents should use this instead of opening QUEUE_STACK.csv.
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 scripts/print_next_queue_item.py "$@"

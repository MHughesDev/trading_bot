#!/usr/bin/env bash
# Close a queue item: mark Done in generate_queue_stack.py, regenerate QUEUE_STACK.csv, optional archive table.
# Usage:
#   bash scripts/queue_close.sh --next
#   bash scripts/queue_close.sh --id FB-CAN-002
#   bash scripts/queue_close.sh --next --dry-run
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 scripts/close_queue_item.py "$@"

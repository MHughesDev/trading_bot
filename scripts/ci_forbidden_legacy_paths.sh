#!/usr/bin/env bash
# FB-CAN-025 — hard-fail if deleted canonical-migration modules reappear or forbidden aliases return.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FORBIDDEN=(
  "decision_engine/action_generator.py"
  "risk_engine/equal_weight.py"
)

for p in "${FORBIDDEN[@]}"; do
  if [[ -e "$p" ]]; then
    echo "ci_forbidden_legacy_paths: forbidden path exists (was removed in canonical migration): $p" >&2
    exit 1
  fi
done

python3 - <<'PY'
import pathlib
import sys
root = pathlib.Path("execution")
for p in root.rglob("*.py"):
    text = p.read_text(encoding="utf-8", errors="replace")
    if "get_execution_adapter" in text:
        print(f"ci_forbidden_legacy_paths: get_execution_adapter in {p} (FB-CAN-021)", file=sys.stderr)
        sys.exit(1)
PY

echo "ci_forbidden_legacy_paths: OK"

#!/usr/bin/env bash
# Fail if Alpaca market-data imports appear outside execution adapters.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

violations=0
while IFS= read -r line; do
  echo "SPEC VIOLATION: $line"
  violations=$((violations + 1))
done < <(
  grep -R -n -E 'alpaca\.(data|data\.historical|data\.live)' --include='*.py' . \
    --exclude-dir='.git' \
    --exclude-dir='legacy' \
    --exclude='alpaca_paper.py' \
    2>/dev/null || true
)

if [[ "$violations" -gt 0 ]]; then
  echo "Coinbase-only market data: remove Alpaca data client imports from these paths."
  exit 1
fi

echo "ci_spec_compliance: OK (no Alpaca market-data imports outside paper adapter)"

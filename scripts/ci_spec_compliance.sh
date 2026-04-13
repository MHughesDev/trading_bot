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
  echo "Kraken-only market data: remove Alpaca data client imports from these paths."
  exit 1
fi

echo "ci_spec_compliance: OK (Kraken-only market data — no Alpaca data imports outside paper adapter)"

# Fail if the legacy/ snapshot package is imported from main V3 code paths (FB-AUD-017 / LEGACY-A1).
legacy_violations=0
while IFS= read -r line; do
  echo "LEGACY IMPORT VIOLATION: $line"
  legacy_violations=$((legacy_violations + 1))
done < <(
  grep -R -n -E '(^from legacy\.|^import legacy\.)' --include='*.py' . \
    --exclude-dir='.git' \
    --exclude-dir='legacy' \
    2>/dev/null || true
)

if [[ "$legacy_violations" -gt 0 ]]; then
  echo "legacy/ isolation: remove imports from legacy/ outside the legacy/ tree."
  exit 1
fi

echo "ci_spec_compliance: OK (legacy/ package not imported from V3 paths)"

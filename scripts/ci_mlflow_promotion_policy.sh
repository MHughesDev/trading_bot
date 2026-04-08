#!/usr/bin/env bash
# Fail if MLflow auto-promotion APIs appear (Master Spec: manual promotion only).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

violations=0
while IFS= read -r line; do
  echo "MLFLOW POLICY: $line"
  violations=$((violations + 1))
done < <(
  grep -R -n -E 'transition_model_version_stage|set_registered_model_alias' \
    --include='*.py' . \
    --exclude-dir='.git' \
    --exclude-dir='.venv' \
    2>/dev/null || true
)

if [[ "$violations" -gt 0 ]]; then
  echo "Remove MLflow staging/alias APIs; use manual promotion (docs/MLFLOW_PROMOTION.md)."
  exit 1
fi

echo "ci_mlflow_promotion_policy: OK"

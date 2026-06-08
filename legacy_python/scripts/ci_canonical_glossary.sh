#!/usr/bin/env bash
# FB-CAN-040 — ensure canonical glossary exists and key docs reference it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

GLOSS="docs/CANONICAL_GLOSSARY.MD"
if [[ ! -f "$GLOSS" ]]; then
  echo "ci_canonical_glossary: missing $GLOSS" >&2
  exit 1
fi

check_ref() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "ci_canonical_glossary: missing $file" >&2
    exit 1
  fi
  if ! grep -q 'CANONICAL_GLOSSARY.MD' "$file"; then
    echo "ci_canonical_glossary: $file must reference CANONICAL_GLOSSARY.MD" >&2
    exit 1
  fi
}

check_ref README.md
check_ref docs/CANONICAL_SPEC_INDEX.MD

echo "ci_canonical_glossary: OK"

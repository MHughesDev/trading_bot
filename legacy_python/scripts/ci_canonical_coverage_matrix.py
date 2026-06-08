#!/usr/bin/env python3
"""FB-CAN-070 — validate canonical spec coverage matrix JSON.

Checks:
- JSON schema_version and required keys
- Each ``implementation_paths`` entry exists under repo root (file or directory)
- Each ``queue_id`` appears in ``scripts/generate_queue_stack.py`` ROWS
- At least one file matches each row's ``test_path_patterns`` (union)

Run from repo root: ``python3 scripts/ci_canonical_coverage_matrix.py``
"""

from __future__ import annotations

import glob
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = ROOT / "docs" / "reports" / "CANONICAL_SPEC_COVERAGE_MATRIX.json"
QUEUE_GEN = ROOT / "scripts" / "generate_queue_stack.py"


def _load_queue_ids_from_generator() -> set[str]:
    text = QUEUE_GEN.read_text(encoding="utf-8")
    return set(re.findall(r'"id":\s*"([^"]+)"', text))


def _path_exists(rel: str) -> bool:
    p = (ROOT / rel.strip()).resolve()
    try:
        p.relative_to(ROOT)
    except ValueError:
        return False
    return p.exists()


def _any_test_matches(patterns: list[str]) -> bool:
    for pat in patterns:
        for g in glob.glob(pat, root_dir=str(ROOT), recursive=True):
            if Path(g).is_file():
                return True
    return False


def main() -> int:
    if not MATRIX_PATH.is_file():
        print(f"ci_canonical_coverage_matrix: missing {MATRIX_PATH}", file=sys.stderr)
        return 1
    try:
        data = json.loads(MATRIX_PATH.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        print(f"ci_canonical_coverage_matrix: invalid JSON: {e}", file=sys.stderr)
        return 1

    if data.get("schema_version") != 1:
        print("ci_canonical_coverage_matrix: schema_version must be 1", file=sys.stderr)
        return 1

    rows = data.get("rows")
    if not isinstance(rows, list) or not rows:
        print("ci_canonical_coverage_matrix: rows must be a non-empty list", file=sys.stderr)
        return 1

    qids_file = _load_queue_ids_from_generator()
    errs: list[str] = []

    for row in rows:
        if not isinstance(row, dict):
            errs.append("row is not an object")
            continue
        rid = row.get("id", "?")
        for rel in row.get("implementation_paths") or []:
            if not isinstance(rel, str) or not rel.strip():
                errs.append(f"{rid}: empty implementation_paths entry")
                continue
            if not _path_exists(rel):
                errs.append(f"{rid}: missing path {rel!r}")
        for qid in row.get("queue_ids") or []:
            if qid not in qids_file:
                errs.append(f"{rid}: queue id {qid!r} not found in generate_queue_stack.py")
        tpat = row.get("test_path_patterns") or []
        if isinstance(tpat, list) and tpat and not _any_test_matches([str(x) for x in tpat]):
            errs.append(f"{rid}: no test files match test_path_patterns {tpat!r}")

    if errs:
        for e in errs:
            print(f"ci_canonical_coverage_matrix: {e}", file=sys.stderr)
        return 1

    print("ci_canonical_coverage_matrix: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

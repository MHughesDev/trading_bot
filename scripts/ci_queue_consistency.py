#!/usr/bin/env python3
"""Verify Open rows in docs/QUEUE_STACK.csv are referenced in docs/QUEUE_ARCHIVE.MD (FB-AUD-008).

Run from repo root: python3 scripts/ci_queue_consistency.py
"""
from __future__ import annotations

import csv
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    csv_path = root / "docs" / "QUEUE_STACK.csv"
    archive_path = root / "docs" / "QUEUE_ARCHIVE.MD"
    text = archive_path.read_text(encoding="utf-8")
    missing: list[str] = []
    with csv_path.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            status = (row.get("status") or "").strip()
            qid = (row.get("id") or "").strip()
            if status != "Open" or qid in ("", "_QUEUE_EMPTY_"):
                continue
            if qid not in text:
                missing.append(qid)
    if missing:
        print(
            "QUEUE consistency: these Open queue IDs are missing from QUEUE_ARCHIVE.MD:",
            ", ".join(missing),
            file=sys.stderr,
        )
        return 1
    print("ci_queue_consistency: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

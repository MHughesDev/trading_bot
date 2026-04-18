#!/usr/bin/env python3
"""Verify Open rows in docs/QUEUE_STACK.csv are referenced in docs/QUEUE_ARCHIVE.MD (FB-AUD-008).

FB-CAN-025: Open rows with batch starting with ``CAN-`` must appear in the §2.7 canonical program
table in QUEUE_ARCHIVE.MD (anchor ``#27-canonical-replacement-program-fb-can``) so canonical
slices stay discoverable.

Run from repo root: python3 scripts/ci_queue_consistency.py
"""
from __future__ import annotations

import csv
import sys
from pathlib import Path

CANONICAL_ANCHOR = "#27-canonical-replacement-program-fb-can"


def check_queue_consistency(csv_path: Path, archive_path: Path) -> tuple[list[str], list[str]]:
    """Return (missing_ids, missing_can_in_canonical_section)."""
    text = archive_path.read_text(encoding="utf-8")
    idx = text.find(CANONICAL_ANCHOR)
    canonical_section = text[idx:] if idx != -1 else text
    missing: list[str] = []
    missing_can_batch: list[str] = []
    with csv_path.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            status = (row.get("status") or "").strip()
            qid = (row.get("id") or "").strip()
            batch = (row.get("batch") or "").strip()
            if status != "Open" or qid in ("", "_QUEUE_EMPTY_"):
                continue
            if qid not in text:
                missing.append(qid)
            if batch.startswith("CAN-") and qid not in canonical_section:
                missing_can_batch.append(qid)
    return missing, missing_can_batch


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    csv_path = root / "docs" / "QUEUE_STACK.csv"
    archive_path = root / "docs" / "QUEUE_ARCHIVE.MD"
    missing, missing_can_batch = check_queue_consistency(csv_path, archive_path)
    if missing:
        print(
            "QUEUE consistency: these Open queue IDs are missing from QUEUE_ARCHIVE.MD:",
            ", ".join(missing),
            file=sys.stderr,
        )
        return 1
    if missing_can_batch:
        print(
            "QUEUE consistency: these Open CAN-* batch IDs are missing from the canonical program "
            f"section ({CANONICAL_ANCHOR}) in QUEUE_ARCHIVE.MD:",
            ", ".join(missing_can_batch),
            file=sys.stderr,
        )
        return 1
    print("ci_queue_consistency: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

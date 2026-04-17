#!/usr/bin/env python3
"""Print the next Open queue item from docs/QUEUE_STACK.csv as one terminal string.

Selection: smallest stack_order among rows where status is Open and id is not _QUEUE_EMPTY_.

Run from repo root:
  python3 scripts/print_next_queue_item.py

Exit codes: 0 if an Open item exists or backlog is empty (sentinel); 2 on CSV/read errors.

See docs/QUEUE.MD and AGENTS.md — agents should run this (or read the CSV) before queue work.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path


def _root() -> Path:
    return Path(__file__).resolve().parents[1]


def _parse_stack_order(raw: str) -> int:
    raw = (raw or "").strip()
    if not raw:
        return 999999
    try:
        return int(raw)
    except ValueError:
        return 999999


def load_next_open_row(csv_path: Path) -> dict[str, str] | None:
    with csv_path.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        if not reader.fieldnames:
            return None
        candidates: list[tuple[int, dict[str, str]]] = []
        for row in reader:
            status = (row.get("status") or "").strip()
            qid = (row.get("id") or "").strip()
            if status != "Open" or qid in ("", "_QUEUE_EMPTY_"):
                continue
            so = _parse_stack_order(row.get("stack_order") or "")
            candidates.append((so, row))
    if not candidates:
        return None
    candidates.sort(key=lambda x: x[0])
    return candidates[0][1]


def format_text(row: dict[str, str]) -> str:
    lines = [
        "=== Next Open queue item (from QUEUE_STACK.csv) ===",
        "",
    ]
    # Stable column order matching typical CSV header; fall back to sorted keys for extras
    preferred = [
        "stack_order",
        "priority",
        "phase",
        "batch",
        "id",
        "kind",
        "status",
        "summary",
        "summary_one_line",
        "agent_task",
        "affected_files",
        "docs_refs",
        "audit_id",
        "anchor",
    ]
    seen: set[str] = set()
    for key in preferred:
        if key not in row:
            continue
        seen.add(key)
        val = (row.get(key) or "").strip()
        lines.append(f"{key}:")
        lines.append(val if val else "(empty)")
        lines.append("")
    for key in sorted(row.keys()):
        if key in seen:
            continue
        val = (row.get(key) or "").strip()
        lines.append(f"{key}:")
        lines.append(val if val else "(empty)")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Print next Open queue item from QUEUE_STACK.csv")
    parser.add_argument(
        "--csv",
        type=Path,
        default=None,
        help="Path to QUEUE_STACK.csv (default: docs/QUEUE_STACK.csv under repo root)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print one JSON object instead of plain text",
    )
    args = parser.parse_args()

    csv_path = args.csv if args.csv is not None else _root() / "docs" / "QUEUE_STACK.csv"
    if not csv_path.is_file():
        print(f"print_next_queue_item: file not found: {csv_path}", file=sys.stderr)
        return 2

    try:
        row = load_next_open_row(csv_path)
    except OSError as e:
        print(f"print_next_queue_item: {e}", file=sys.stderr)
        return 2

    if row is None:
        msg = (
            "QUEUE_EMPTY: No Open rows in QUEUE_STACK.csv "
            "(or only _QUEUE_EMPTY_ sentinel). Nothing to implement — see docs/QUEUE.MD §6."
        )
        print(msg)
        return 0

    if args.json:
        print(json.dumps(row, ensure_ascii=False, indent=2))
    else:
        sys.stdout.write(format_text(row))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

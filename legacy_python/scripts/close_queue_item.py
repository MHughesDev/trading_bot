#!/usr/bin/env python3
"""Close a queue item: set status Done in scripts/generate_queue_stack.py and regenerate CSV.

Optionally updates the narrative table row in docs/QUEUE_ARCHIVE.MD (Open -> Done) when the
ID appears in a pipe table line (e.g. §2.7 canonical program tables).

Run from repo root:
  python3 scripts/close_queue_item.py --next
  python3 scripts/close_queue_item.py --id FB-CAN-002

Agents should use this (or scripts/queue_close.sh) instead of loading QUEUE_STACK.csv or
QUEUE_ARCHIVE.MD in full. See docs/QUEUE.MD and docs/QUEUE_SCHEMA.md.

Exit codes: 0 success, 1 usage/logic error, 2 file/read error.
"""
from __future__ import annotations

import argparse
import csv
import subprocess
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
    """Same selection as print_next_queue_item.py (smallest stack_order, status Open)."""
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


def _replace_status_in_dict_block(content: str, target_id: str, new_status: str) -> tuple[str, bool]:
    """Find the ROWS dict entry with "id": "<target_id>" and set "status": "<new_status>"."""
    needle = f'"id": "{target_id}"'
    i = content.find(needle)
    if i == -1:
        raise ValueError(f'ROWS entry not found for id "{target_id}" in generator file')
    j = content.rfind("{", 0, i)
    if j == -1:
        raise ValueError("Could not locate dict opening brace for id block")

    depth = 0
    k = j
    while k < len(content):
        c = content[k]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                block = content[j : k + 1]
                old_open = '"status": "Open"'
                if old_open not in block:
                    raise ValueError(
                        f'Entry "{target_id}" has no "status": "Open" in generator (already closed?)'
                    )
                new_block = block.replace(old_open, f'"status": "{new_status}"', 1)
                return content[:j] + new_block + content[k + 1 :], True
        k += 1
    raise ValueError("Unclosed dict block while closing queue item")


def _archive_table_line_open_to_done(line: str, target_id: str) -> str | None:
    """If line is a pipe row for target_id with status Open, return line with Open -> Done."""
    if "|" not in line or target_id not in line:
        return None
    parts = line.split("|")
    if len(parts) < 9:
        return None
    if parts[1].strip() != target_id:
        return None
    # Column order: ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | ...
    status = parts[7].strip()
    if status != "Open":
        return None
    parts[7] = " Done "
    return "|".join(parts)


def _update_archive_md(path: Path, target_id: str) -> bool:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    changed = False
    out: list[str] = []
    for line in lines:
        new_line = _archive_table_line_open_to_done(line, target_id)
        if new_line is not None:
            out.append(new_line if new_line.endswith("\n") else new_line + "\n")
            changed = True
        else:
            out.append(line)
    if changed:
        path.write_text("".join(out), encoding="utf-8")
    return changed


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Close a queue item (generator ROWS + regenerate CSV; optional archive table)"
    )
    g = parser.add_mutually_exclusive_group(required=True)
    g.add_argument("--id", type=str, help="Queue id to close (e.g. FB-CAN-002)")
    g.add_argument(
        "--next",
        action="store_true",
        help="Close the same next Open row as print_next_queue_item (smallest stack_order)",
    )
    parser.add_argument(
        "--generator",
        type=Path,
        default=None,
        help="Path to generate_queue_stack.py (default: scripts/generate_queue_stack.py)",
    )
    parser.add_argument(
        "--archive-md",
        type=Path,
        default=None,
        help="Path to QUEUE_ARCHIVE.MD (default: docs/QUEUE_ARCHIVE.MD); use --skip-archive to skip",
    )
    parser.add_argument(
        "--skip-archive",
        action="store_true",
        help="Do not try to flip Open -> Done in QUEUE_ARCHIVE.MD",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print actions only; do not write files or run generator",
    )
    args = parser.parse_args()

    root = _root()
    csv_path = root / "docs" / "QUEUE_STACK.csv"
    gen_path = args.generator if args.generator is not None else root / "scripts" / "generate_queue_stack.py"
    archive_path = args.archive_md if args.archive_md is not None else root / "docs" / "QUEUE_ARCHIVE.MD"

    target_id: str
    if args.next:
        if not csv_path.is_file():
            print(f"close_queue_item: file not found: {csv_path}", file=sys.stderr)
            return 2
        try:
            row = load_next_open_row(csv_path)
        except OSError as e:
            print(f"close_queue_item: {e}", file=sys.stderr)
            return 2
        if row is None:
            print(
                "close_queue_item: no Open row to close (QUEUE_EMPTY).",
                file=sys.stderr,
            )
            return 1
        target_id = (row.get("id") or "").strip()
        if not target_id or target_id == "_QUEUE_EMPTY_":
            print("close_queue_item: invalid next row id.", file=sys.stderr)
            return 1
    else:
        target_id = (args.id or "").strip()
        if not target_id:
            print("close_queue_item: --id is empty.", file=sys.stderr)
            return 1

    if not gen_path.is_file():
        print(f"close_queue_item: generator not found: {gen_path}", file=sys.stderr)
        return 2

    try:
        content = gen_path.read_text(encoding="utf-8")
    except OSError as e:
        print(f"close_queue_item: {e}", file=sys.stderr)
        return 2

    try:
        new_content, _ = _replace_status_in_dict_block(content, target_id, "Done")
    except ValueError as e:
        print(f"close_queue_item: {e}", file=sys.stderr)
        return 1

    if args.dry_run:
        print(f"Would set id={target_id} status=Done in {gen_path}")
        if not args.skip_archive and archive_path.is_file():
            print(f"Would try Open->Done for id={target_id} in {archive_path}")
        print("Would run: python3 scripts/generate_queue_stack.py")
        return 0

    gen_path.write_text(new_content, encoding="utf-8")

    rc = subprocess.run(
        [sys.executable, str(root / "scripts" / "generate_queue_stack.py")],
        cwd=root,
        check=False,
    )
    if rc.returncode != 0:
        print("close_queue_item: generate_queue_stack.py failed.", file=sys.stderr)
        return 2

    if not args.skip_archive and archive_path.is_file():
        updated = _update_archive_md(archive_path, target_id)
        if updated:
            print(f"close_queue_item: updated table row for {target_id} in {archive_path}")
        else:
            print(
                f"close_queue_item: no matching Open table row for {target_id} in {archive_path} "
                "(mirror manually if needed).",
                file=sys.stderr,
            )

    print(f"close_queue_item: closed {target_id} (status=Done, CSV regenerated).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

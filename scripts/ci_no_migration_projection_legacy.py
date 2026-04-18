#!/usr/bin/env python3
"""FB-CAN-060 — fail if migration-only ``projection: legacy`` markers reappear in code.

The canonical config baseline is synthesized from AppSettings; markers were removed in FB-CAN-060.
Run from repo root: python3 scripts/ci_no_migration_projection_legacy.py
"""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKIP_DIRS = frozenset(
    {
        ".git",
        ".venv",
        "venv",
        "node_modules",
        "legacy",
        "__pycache__",
        ".ruff_cache",
        ".pytest_cache",
    }
)
# Built dynamically so this guard file does not trip its own scan.
NEEDLE = "".join(['"', "projection", '"', ": ", '"', "legacy", '"'])


def main() -> int:
    bad: list[str] = []
    for p in ROOT.rglob("*"):
        if not p.is_file():
            continue
        if any(part in SKIP_DIRS for part in p.parts):
            continue
        if p.suffix not in {".py", ".yaml", ".yml", ".md"}:
            continue
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if NEEDLE in text:
            bad.append(str(p.relative_to(ROOT)))
    if bad:
        print(
            "ci_no_migration_projection_legacy: forbidden "
            f"{NEEDLE!r} found in:\n  " + "\n  ".join(sorted(bad)),
            file=sys.stderr,
        )
        return 1
    print("ci_no_migration_projection_legacy: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

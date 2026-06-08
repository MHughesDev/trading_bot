#!/usr/bin/env python3
"""Print deployment preflight JSON (IL-105 / FB-SPEC-08). Exit 1 if preflight reports ok=false."""

from __future__ import annotations

import json
import sys
from pathlib import Path

# Repo root on PYTHONPATH when run as `python scripts/preflight_check.py`
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.config.settings import load_settings
from control_plane.preflight import preflight_report


def main() -> int:
    s = load_settings()
    r = preflight_report(s)
    print(json.dumps(r, indent=2))
    return 0 if r.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())

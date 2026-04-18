"""FB-CAN-067: canonical magic constant CI guard."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def test_ci_canonical_magic_constants_exits_zero():
    r = subprocess.run(
        [sys.executable, str(REPO / "scripts" / "ci_canonical_magic_constants.py")],
        cwd=str(REPO),
        capture_output=True,
        text=True,
        check=False,
    )
    assert r.returncode == 0, r.stderr or r.stdout

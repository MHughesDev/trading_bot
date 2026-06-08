"""FB-CAN-070: canonical coverage matrix CI script exits zero."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def test_ci_canonical_coverage_matrix_exits_zero() -> None:
    root = Path(__file__).resolve().parents[1]
    script = root / "scripts" / "ci_canonical_coverage_matrix.py"
    proc = subprocess.run(
        [sys.executable, str(script)],
        cwd=str(root),
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr or proc.stdout

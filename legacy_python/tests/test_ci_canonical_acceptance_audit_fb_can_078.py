"""FB-CAN-078 canonical acceptance audit script."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci_canonical_acceptance_audit.py"
REPORT = ROOT / "docs" / "reports" / "CANONICAL_ACCEPTANCE_AUDIT_REPORT.json"


def test_acceptance_audit_skips_subprocess_in_ci_mode() -> None:
    env = {**os.environ, "NM_ACCEPTANCE_AUDIT_SKIP_SUBPROCESS": "1"}
    r = subprocess.run(
        [sys.executable, str(SCRIPT), "--no-write-report"],
        cwd=str(ROOT),
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert r.returncode == 0, r.stderr + r.stdout


def test_acceptance_audit_writes_json_report(tmp_path: Path) -> None:
    out = tmp_path / "report.json"
    env = {**os.environ, "NM_ACCEPTANCE_AUDIT_SKIP_SUBPROCESS": "1"}
    r = subprocess.run(
        [sys.executable, str(SCRIPT), "--json-out", str(out)],
        cwd=str(ROOT),
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert r.returncode == 0, r.stderr + r.stdout
    data = json.loads(out.read_text(encoding="utf-8"))
    assert data.get("schema_version") == 1
    assert data.get("audit_id") == "CANONICAL-ACCEPTANCE-AUDIT"
    assert data.get("ok") is True
    assert "steps" in data


@pytest.mark.skipif(not REPORT.is_file(), reason="report not generated yet")
def test_checked_in_acceptance_report_is_valid_json() -> None:
    json.loads(REPORT.read_text(encoding="utf-8"))

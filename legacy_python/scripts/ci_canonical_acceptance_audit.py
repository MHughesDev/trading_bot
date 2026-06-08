#!/usr/bin/env python3
"""FB-CAN-078 — canonical migration acceptance audit (pre-closeout gate).

Runs **after** ``ci_canonical_gates_inner.sh`` (replay determinism, coverage matrix, magic constants, …).
This script adds:

  - Release-gate / rollback validation (``ci_canonical_contracts.sh`` — governance evidence fixtures)
  - Required audit artifacts on disk (gap audit markdown, human ``canonical/*.md`` spec tree)
  - Machine-readable report under ``docs/reports/CANONICAL_ACCEPTANCE_AUDIT_REPORT.json``

**Open queue:** by default, **FB-CAN-*** items still **Open** in ``docs/QUEUE_STACK.csv`` are reported
as warnings. Use ``--strict-open-queue`` to fail if any remain (final migration closeout).

Environment:
  ``NM_ACCEPTANCE_AUDIT_SKIP_SUBPROCESS=1`` — skip ``ci_canonical_contracts.sh`` (unit tests only).
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
MATRIX_PATH = ROOT / "docs" / "reports" / "CANONICAL_SPEC_COVERAGE_MATRIX.json"
GAP_AUDIT_PATH = ROOT / "docs" / "reports" / "REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md"
CANONICAL_SPECS_DIR = ROOT / "docs" / "Human Provided Specs" / "new_specs" / "canonical"
QUEUE_CSV = ROOT / "docs" / "QUEUE_STACK.csv"
REPORT_JSON = ROOT / "docs" / "reports" / "CANONICAL_ACCEPTANCE_AUDIT_REPORT.json"
FIXTURES_REQUIRED = (
    "tests/fixtures/canonical_release_candidate_live.json",
    "tests/fixtures/canonical_experiment_registry_ci.json",
)


def _run(cmd: list[str], *, cwd: Path) -> tuple[int, str]:
    p = subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    out = (p.stdout or "") + (p.stderr or "")
    return p.returncode, out.strip()


def _load_matrix() -> dict[str, Any]:
    if not MATRIX_PATH.is_file():
        raise FileNotFoundError(str(MATRIX_PATH))
    return json.loads(MATRIX_PATH.read_text(encoding="utf-8"))


def _open_fb_can_queue_ids() -> list[str]:
    if not QUEUE_CSV.is_file():
        return []
    out: list[str] = []
    with QUEUE_CSV.open(encoding="utf-8", newline="") as f:
        for row in csv.DictReader(f):
            qid = (row.get("id") or "").strip()
            st = (row.get("status") or "").strip()
            if st != "Open" or not qid or qid == "_QUEUE_EMPTY_":
                continue
            if qid.startswith("FB-CAN-"):
                out.append(qid)
    return sorted(set(out))


def _canonical_spec_md_count() -> int:
    if not CANONICAL_SPECS_DIR.is_dir():
        return 0
    return len(list(CANONICAL_SPECS_DIR.glob("*.md")))


def main() -> int:
    ap = argparse.ArgumentParser(description="FB-CAN-078 canonical acceptance audit")
    ap.add_argument(
        "--strict-open-queue",
        action="store_true",
        help="Fail if any FB-CAN-* queue rows are still Open (migration closeout)",
    )
    ap.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help=f"Write machine-readable report (default: {REPORT_JSON})",
    )
    ap.add_argument(
        "--no-write-report",
        action="store_true",
        help="Do not write CANONICAL_ACCEPTANCE_AUDIT_REPORT.json",
    )
    args = ap.parse_args()

    errs: list[str] = []
    warnings: list[str] = []
    steps: list[dict[str, Any]] = []

    skip_sub = os.environ.get("NM_ACCEPTANCE_AUDIT_SKIP_SUBPROCESS", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )

    # --- Static prerequisites ---
    if not GAP_AUDIT_PATH.is_file():
        errs.append(f"missing gap audit: {GAP_AUDIT_PATH.relative_to(ROOT)}")
    else:
        steps.append({"step": "gap_audit_doc", "ok": True, "path": str(GAP_AUDIT_PATH.relative_to(ROOT))})

    n_specs = _canonical_spec_md_count()
    if n_specs < 1:
        errs.append(
            f"expected at least one *.md under {CANONICAL_SPECS_DIR.relative_to(ROOT)} (canonical spec tree)"
        )
    else:
        steps.append({"step": "canonical_spec_tree", "ok": True, "md_count": n_specs})

    for rel in FIXTURES_REQUIRED:
        p = ROOT / rel
        if not p.is_file():
            errs.append(f"missing governance fixture: {rel}")
        else:
            steps.append({"step": "fixture", "ok": True, "path": rel})

    try:
        matrix = _load_matrix()
    except (OSError, json.JSONDecodeError) as e:
        errs.append(f"coverage matrix: {e}")
        matrix = {}

    checklist = matrix.get("migration_signoff_checklist")
    if isinstance(checklist, list):
        steps.append({"step": "matrix_signoff_checklist", "ok": True, "items": len(checklist)})
    else:
        warnings.append("coverage matrix has no migration_signoff_checklist array")

    open_can = _open_fb_can_queue_ids()
    if open_can:
        msg = f"Open canonical queue items ({len(open_can)}): {', '.join(open_can)}"
        if args.strict_open_queue:
            errs.append(msg)
        else:
            warnings.append(msg)
    steps.append(
        {
            "step": "open_fb_can_queue",
            "count": len(open_can),
            "ids": open_can,
            "strict": bool(args.strict_open_queue),
        }
    )

    # --- Governance / contract gates (release candidate fixture + rollback playbook) ---
    if not skip_sub:
        rc2, out2 = _run(["bash", str(ROOT / "scripts" / "ci_canonical_contracts.sh")], cwd=ROOT)
        steps.append({"step": "ci_canonical_contracts", "exit_code": rc2, "ok": rc2 == 0})
        if rc2 != 0:
            errs.append(f"ci_canonical_contracts failed:\n{out2[:4000]}")
    else:
        steps.append({"step": "ci_canonical_contracts", "skipped": True})

    report: dict[str, Any] = {
        "schema_version": 1,
        "audit_id": "CANONICAL-ACCEPTANCE-AUDIT",
        "queue_id": "FB-CAN-078",
        "generated_at_utc": datetime.now(UTC).replace(microsecond=0).isoformat(),
        "ok": len(errs) == 0,
        "errors": errs,
        "warnings": warnings,
        "steps": steps,
    }

    if not args.no_write_report:
        out_path = args.json_out or REPORT_JSON
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(report, indent=2, sort_keys=False) + "\n", encoding="utf-8")

    for w in warnings:
        print(f"ci_canonical_acceptance_audit: WARNING: {w}", file=sys.stderr)

    if errs:
        for e in errs:
            print(f"ci_canonical_acceptance_audit: {e}", file=sys.stderr)
        print("ci_canonical_acceptance_audit: FAIL", file=sys.stderr)
        return 1

    print("ci_canonical_acceptance_audit: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

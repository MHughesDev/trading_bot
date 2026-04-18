"""FB-CAN-025: queue consistency flags Open CAN-* rows missing from canonical §2.7 section."""

from __future__ import annotations

import importlib.util
from pathlib import Path


def _load_check():
    root = Path(__file__).resolve().parents[1]
    path = root / "scripts" / "ci_queue_consistency.py"
    spec = importlib.util.spec_from_file_location("ci_queue_consistency", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_check_queue_consistency_flags_can_batch_not_in_canonical_section(tmp_path: Path) -> None:
    mod = _load_check()
    csv_path = tmp_path / "QUEUE_STACK.csv"
    archive_path = tmp_path / "QUEUE_ARCHIVE.MD"
    csv_path.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary\n"
        "1,HIGH,B,CAN-TEST,FB-CAN-888,feature,Open,slice\n",
        encoding="utf-8",
    )
    archive_path.write_text(
        "# Archive\n\n## 2. Open queue\n\n"
        "| ID | Batch | Status |\n"
        "|----|-------|--------|\n"
        "| FB-CAN-888 | CAN-TEST | Open |\n\n"
        f"## 2.7 Canonical program {mod.CANONICAL_ANCHOR}\n\n"
        "Narrative only — this slice is intentionally omitted from the table below.\n",
        encoding="utf-8",
    )

    missing, missing_can = mod.check_queue_consistency(csv_path, archive_path)
    assert missing == []
    assert missing_can == ["FB-CAN-888"]


def test_check_queue_consistency_ok_when_can_id_in_canonical_section(tmp_path: Path) -> None:
    mod = _load_check()
    csv_path = tmp_path / "QUEUE_STACK.csv"
    archive_path = tmp_path / "QUEUE_ARCHIVE.MD"
    csv_path.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary\n"
        "1,HIGH,B,CAN-TEST,FB-CAN-888,feature,Open,slice\n",
        encoding="utf-8",
    )
    archive_path.write_text(
        f"## 2.7 x {mod.CANONICAL_ANCHOR}\n\n| FB-CAN-888 |\n",
        encoding="utf-8",
    )

    missing, missing_can = mod.check_queue_consistency(csv_path, archive_path)
    assert missing == []
    assert missing_can == []

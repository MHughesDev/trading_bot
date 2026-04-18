"""Tests for scripts/close_queue_item.py helpers."""

from __future__ import annotations

import importlib.util
from pathlib import Path


def _load_module():
    root = Path(__file__).resolve().parents[1]
    path = root / "scripts" / "close_queue_item.py"
    spec = importlib.util.spec_from_file_location("close_queue_item", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_replace_status_in_dict_block():
    mod = _load_module()
    src = """
ROWS: list[dict[str, str]] = [
    {
        "id": "FB-A",
        "status": "Open",
    },
    {
        "id": "FB-B",
        "status": "Open",
    },
]
"""
    out, _ok = mod._replace_status_in_dict_block(src, "FB-B", "Done")
    i = out.index('"id": "FB-B"')
    assert '"status": "Done"' in out[i : i + 120]
    i_a = out.index('"id": "FB-A"')
    assert '"status": "Open"' in out[i_a : i_a + 120]


def test_archive_table_line_open_to_done():
    mod = _load_module()
    line = "| FB-X | B | A | OP | change | HIGH | Open | Summary here | files |"
    new_line = mod._archive_table_line_open_to_done(line, "FB-X")
    assert new_line is not None
    assert "| Done |" in new_line
    assert mod._archive_table_line_open_to_done(line, "FB-Y") is None

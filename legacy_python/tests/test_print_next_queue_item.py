"""Tests for scripts/print_next_queue_item.py"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

from scripts.print_next_queue_item import format_text, load_next_open_row, main


def test_load_next_open_row_smallest_stack_order(tmp_path: Path) -> None:
    p = tmp_path / "q.csv"
    p.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task\n"
        "2,MEDIUM,B,,B-2,x,Open,s2,s2,task2\n"
        "1,HIGH,A,,A-1,x,Open,s1,s1,task1\n",
        encoding="utf-8",
    )
    row = load_next_open_row(p)
    assert row is not None
    assert row["id"] == "A-1"
    assert row["stack_order"] == "1"


def test_load_next_open_row_skips_done_and_sentinel(tmp_path: Path) -> None:
    p = tmp_path / "q.csv"
    p.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task\n"
        "1,LOW,D,,_QUEUE_EMPTY_,deferred,empty,x,x,\n"
        "2,MEDIUM,B,,X,x,Done,s,s,t\n"
        "3,HIGH,A,,Y,y,Open,s,s,doit\n",
        encoding="utf-8",
    )
    row = load_next_open_row(p)
    assert row is not None
    assert row["id"] == "Y"


def test_load_next_open_row_none_when_empty(tmp_path: Path) -> None:
    p = tmp_path / "q.csv"
    p.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task\n"
        "1,LOW,D,,_QUEUE_EMPTY_,deferred,empty,x,x,\n",
        encoding="utf-8",
    )
    assert load_next_open_row(p) is None


def test_format_text_includes_agent_task() -> None:
    row = {
        "stack_order": "1",
        "id": "FB-1",
        "agent_task": "Do the thing",
        "status": "Open",
    }
    out = format_text(row)
    assert "FB-1" in out
    assert "Do the thing" in out


def test_cli_json_stdout(monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    p = tmp_path / "q.csv"
    p.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task\n"
        "1,HIGH,A,,CLI-1,x,Open,s,s,task\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(sys, "argv", ["print_next_queue_item.py", "--csv", str(p), "--json"])
    assert main() == 0
    captured = capsys.readouterr().out
    data = json.loads(captured)
    assert data.get("id") == "CLI-1"
    assert data.get("status") == "Open"


def test_cli_json_stdout_queue_empty(monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path) -> None:
    p = tmp_path / "empty.csv"
    p.write_text(
        "stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task\n"
        "1,LOW,D,,_QUEUE_EMPTY_,deferred,empty,x,x,\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(sys, "argv", ["print_next_queue_item.py", "--csv", str(p), "--json"])
    assert main() == 0
    data = json.loads(capsys.readouterr().out)
    assert data.get("queue_empty") is True
    assert "message" in data

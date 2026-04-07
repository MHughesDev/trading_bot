"""Live and replay must share decision_engine.run_step."""

from __future__ import annotations

import ast
from pathlib import Path


def test_live_service_imports_run_decision_tick():
    root = Path(__file__).resolve().parents[1]
    src = (root / "app" / "runtime" / "live_service.py").read_text(encoding="utf-8")
    tree = ast.parse(src)
    names = {
        n.name
        for n in tree.body
        if isinstance(n, ast.ImportFrom) and n.module == "decision_engine.run_step"
        for n in n.names
    }
    assert "run_decision_tick" in names


def test_replay_imports_run_decision_tick():
    root = Path(__file__).resolve().parents[1]
    src = (root / "backtesting" / "replay.py").read_text(encoding="utf-8")
    assert "run_decision_tick" in src

"""Operator launcher finds repo root (packaging/windows/trading_bot_operator_launcher.py)."""

from __future__ import annotations

import importlib.util
from pathlib import Path


def _load_launcher():
    root = Path(__file__).resolve().parents[1]
    path = root / "packaging" / "windows" / "trading_bot_operator_launcher.py"
    spec = importlib.util.spec_from_file_location("trading_bot_operator_launcher", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_find_repo_root_from_cwd(tmp_path, monkeypatch) -> None:
    (tmp_path / "setup.bat").write_text("@echo off\n", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    m = _load_launcher()
    assert m._find_repo_root() == tmp_path.resolve()

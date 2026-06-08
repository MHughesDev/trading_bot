"""Cross-platform desktop shortcut builders + installer."""

from __future__ import annotations

import os
import stat
from pathlib import Path

from operator_packaging.desktop_app import shortcuts


def test_linux_desktop_entry_contains_exec_and_path():
    entry = shortcuts.linux_desktop_entry(
        python_exe="/repo/.venv/bin/python", repo_root="/repo", icon_path="/repo/icon.svg"
    )
    assert "[Desktop Entry]" in entry
    assert "Exec=/repo/.venv/bin/python -m operator_packaging.desktop_app" in entry
    assert "Path=/repo" in entry
    assert "Icon=/repo/icon.svg" in entry
    assert "Terminal=false" in entry


def test_linux_desktop_entry_without_icon_omits_icon_line():
    entry = shortcuts.linux_desktop_entry(python_exe="py", repo_root="/r")
    assert "Icon=" not in entry


def test_macos_command_script_is_bash_and_execs_module():
    script = shortcuts.macos_command_script(python_exe="/v/python", repo_root="/r")
    assert script.startswith("#!/bin/bash")
    assert 'cd "/r"' in script
    assert "operator_packaging.desktop_app" in script


def test_windows_vbs_runs_hidden():
    vbs = shortcuts.windows_vbs_script(pythonw_exe="C:\\py\\pythonw.exe", repo_root="C:\\repo")
    # The 0 window-style argument => no console window.
    assert ", 0, False" in vbs
    assert "operator_packaging.desktop_app" in vbs


def test_install_desktop_shortcut_linux(tmp_path: Path):
    dest = tmp_path / "Desktop"
    target = shortcuts.install_desktop_shortcut(
        python_exe="/v/python",
        repo_root=tmp_path,
        dest_dir=dest,
        system="Linux",
    )
    assert target.name == "Trading Bot.desktop"
    assert target.exists()
    assert os.access(target, os.X_OK)


def test_install_desktop_shortcut_macos_executable(tmp_path: Path):
    target = shortcuts.install_desktop_shortcut(
        python_exe="/v/python",
        repo_root=tmp_path,
        dest_dir=tmp_path / "Desktop",
        system="Darwin",
    )
    assert target.name == "Trading Bot.command"
    assert target.stat().st_mode & stat.S_IXUSR


def test_install_desktop_shortcut_windows(tmp_path: Path):
    target = shortcuts.install_desktop_shortcut(
        python_exe="C:\\py\\python.exe",
        repo_root=tmp_path,
        dest_dir=tmp_path / "Desktop",
        system="Windows",
    )
    assert target.name == "Trading Bot.vbs"
    assert target.exists()

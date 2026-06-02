"""Cross-platform desktop shortcut generation for the one-click launcher.

The string builders are pure (testable); :func:`install_desktop_shortcut` picks
the right one for the current OS and writes it to the user's Desktop. Nothing
here imports a GUI toolkit.
"""

from __future__ import annotations

import os
import platform
import stat
import sys
from pathlib import Path

APP_NAME = "Trading Bot"
_LAUNCH_MODULE = "operator_packaging.desktop_app"


def desktop_dir() -> Path:
    """Best-effort path to the user's Desktop (falls back to home)."""
    home = Path.home()
    candidate = home / "Desktop"
    return candidate if candidate.is_dir() else home


def linux_desktop_entry(
    *, python_exe: str, repo_root: str, icon_path: str | None = None
) -> str:
    """A freedesktop ``.desktop`` launcher entry (Linux)."""
    icon_line = f"Icon={icon_path}\n" if icon_path else ""
    return (
        "[Desktop Entry]\n"
        "Type=Application\n"
        f"Name={APP_NAME}\n"
        "Comment=Start the Trading Bot and open the dashboard\n"
        f'Exec={python_exe} -m {_LAUNCH_MODULE}\n'
        f"Path={repo_root}\n"
        f"{icon_line}"
        "Terminal=false\n"
        "Categories=Office;Finance;\n"
    )


def macos_command_script(*, python_exe: str, repo_root: str) -> str:
    """A double-clickable ``.command`` script (macOS)."""
    return (
        "#!/bin/bash\n"
        "# Trading Bot — one-click desktop launcher (macOS).\n"
        f'cd "{repo_root}" || exit 1\n'
        f'exec "{python_exe}" -m {_LAUNCH_MODULE}\n'
    )


def windows_vbs_script(*, pythonw_exe: str, repo_root: str) -> str:
    """A ``.vbs`` that launches with no console window (Windows).

    Uses ``pythonw.exe`` so the operator sees only the app window, not a console.
    """
    return (
        "' Trading Bot — one-click desktop launcher (Windows).\n"
        "Set sh = CreateObject(\"WScript.Shell\")\n"
        f'sh.CurrentDirectory = "{repo_root}"\n'
        f'sh.Run """{pythonw_exe}"" -m {_LAUNCH_MODULE}", 0, False\n'
    )


def _pythonw_for(python_exe: str) -> str:
    """Windows: prefer ``pythonw.exe`` (no console) next to ``python.exe``."""
    p = Path(python_exe)
    cand = p.with_name("pythonw.exe")
    return str(cand) if cand.exists() else python_exe


def _icon_path_or_none(repo_root: Path) -> str | None:
    svg = repo_root / "operator_packaging" / "desktop_app" / "assets" / "icon.svg"
    return str(svg) if svg.exists() else None


def install_desktop_shortcut(
    *,
    python_exe: str | None = None,
    repo_root: Path | None = None,
    dest_dir: Path | None = None,
    system: str | None = None,
) -> Path:
    """Write an OS-appropriate shortcut to the Desktop; returns its path."""
    from operator_packaging.desktop_app.launcher import repo_root as _detect_root
    from operator_packaging.desktop_app.launcher import resolve_python_exe

    root = repo_root or _detect_root()
    py = python_exe or resolve_python_exe(root)
    dest = dest_dir or desktop_dir()
    osname = (system or platform.system()).lower()
    dest.mkdir(parents=True, exist_ok=True)

    if osname.startswith("win"):
        target = dest / f"{APP_NAME}.vbs"
        target.write_text(
            windows_vbs_script(pythonw_exe=_pythonw_for(py), repo_root=str(root)),
            encoding="utf-8",
        )
        return target

    if osname == "darwin":
        target = dest / f"{APP_NAME}.command"
        target.write_text(macos_command_script(python_exe=py, repo_root=str(root)), encoding="utf-8")
        _make_executable(target)
        return target

    # Linux / other freedesktop.
    target = dest / f"{APP_NAME}.desktop"
    target.write_text(
        linux_desktop_entry(python_exe=py, repo_root=str(root), icon_path=_icon_path_or_none(root)),
        encoding="utf-8",
    )
    _make_executable(target)
    return target


def _make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def main() -> int:
    target = install_desktop_shortcut()
    print(f"Created desktop shortcut: {target}")
    if os.name != "nt":
        print("If your file manager flags it, mark it 'Allow launching' / trusted once.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

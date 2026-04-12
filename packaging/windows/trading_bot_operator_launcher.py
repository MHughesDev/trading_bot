"""
Windows operator launcher — same process layout as ``run.bat`` (FB-UI-01-01).

Spawns: uvicorn control plane, power supervisor, Streamlit dashboard.
Requires a repo clone with ``setup.bat`` run (``.venv`` present). Intended to be
frozen with PyInstaller; the executable must live **in the repo root** next to
``setup.bat``, or run with current directory set to the repo root (the launcher
searches upward for ``setup.bat``).
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
from pathlib import Path


def _find_repo_root() -> Path:
    """Locate clone root (directory containing ``setup.bat``)."""
    candidates: list[Path] = []
    candidates.append(Path.cwd().resolve())
    if getattr(sys, "frozen", False):
        candidates.append(Path(sys.executable).resolve().parent)
    else:
        here = Path(__file__).resolve()
        candidates.append(here.parent.parent)  # packaging/windows -> repo root

    seen: set[Path] = set()
    for start in candidates:
        if start in seen:
            continue
        seen.add(start)
        for p in [start, *start.parents]:
            if (p / "setup.bat").is_file():
                return p
    raise RuntimeError(
        "Could not find repository root (setup.bat). "
        "Run this program from your Trading Bot clone directory, "
        "or place trading_bot_operator_launcher.exe next to setup.bat."
    )


def main() -> None:
    root = _find_repo_root()
    os.chdir(root)

    vpy = root / ".venv" / "Scripts" / "python.exe"
    if not vpy.is_file():
        sys.stderr.write("ERROR: .venv not found. Run setup.bat first.\n")
        sys.exit(1)

    creationflags = 0
    if sys.platform == "win32":
        creationflags = subprocess.CREATE_NEW_CONSOLE  # type: ignore[attr-defined]

    def _popen(args: list[str]) -> subprocess.Popen[bytes]:
        return subprocess.Popen(
            args,
            cwd=str(root),
            creationflags=creationflags,
        )

    _popen(
        [
            str(vpy),
            "-m",
            "uvicorn",
            "control_plane.api:app",
            "--host",
            "127.0.0.1",
            "--port",
            "8000",
        ],
    )
    time.sleep(2)
    _popen([str(vpy), "-m", "app.runtime.power_supervisor"])
    _popen(
        [
            str(vpy),
            "-m",
            "streamlit",
            "run",
            str(root / "control_plane" / "Home.py"),
            "--server.headless",
            "true",
        ],
    )

    print()
    print("Started (same as run.bat):")
    print("  - Control plane: http://127.0.0.1:8000")
    print("  - Supervisor: live runtime on :8208 when power ON")
    print("  - Dashboard: Streamlit (usually http://localhost:8501)")
    print()
    print("Close the spawned console windows to stop.")
    print("Set NM_POWER_SUPERVISOR_ENABLED=false to skip auto live runtime.")


if __name__ == "__main__":
    main()

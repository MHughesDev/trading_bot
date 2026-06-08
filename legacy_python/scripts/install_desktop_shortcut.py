#!/usr/bin/env python3
"""Create a clickable desktop shortcut for the Trading Bot launcher.

Usage (from repo root, after ``pip install -e ".[dashboard]"``):

    python scripts/install_desktop_shortcut.py

Detects your OS and writes a ``.vbs`` (Windows), ``.command`` (macOS), or
``.desktop`` (Linux) file to your Desktop that runs
``python -m operator_packaging.desktop_app``.
"""

from __future__ import annotations

from operator_packaging.desktop_app.shortcuts import main

if __name__ == "__main__":
    raise SystemExit(main())

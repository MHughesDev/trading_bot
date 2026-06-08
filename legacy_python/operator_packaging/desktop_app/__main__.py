"""``python -m operator_packaging.desktop_app`` entrypoint."""

from __future__ import annotations

from operator_packaging.desktop_app.launcher import main

if __name__ == "__main__":
    raise SystemExit(main())

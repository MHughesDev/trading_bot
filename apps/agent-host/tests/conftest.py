"""Put the package on the path without requiring an editable install.

`agent_host` lives under `src/`, so importing it needs either `pip install -e .` or
this. CI, a fresh clone and a developer running `pytest` from the repository root all
hit the same `ModuleNotFoundError` otherwise, and the failure looks like a broken
test rather than a missing install step.
"""

from __future__ import annotations

import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

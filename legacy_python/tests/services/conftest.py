"""Default env for microservice scaffold tests (no live venue)."""

from __future__ import annotations

import os

# Execution gateway: scaffold ack/fill unless a test opts into stub+submit.
os.environ.setdefault("NM_EXECUTION_GATEWAY_SUBMIT", "false")

"""Power supervisor start/stop behavior (mocked subprocess)."""

from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock

import pytest

from app.runtime import power_supervisor as ps
from app.runtime import system_power as sp


@pytest.fixture
def isolated_power_state(monkeypatch, tmp_path: Path):
    monkeypatch.setattr(sp, "_STATE_PATH", tmp_path / "system_power.json")
    sp.set_power("on")
    yield
    sp.set_power("on")


def test_supervisor_starts_then_stops_live_runtime(monkeypatch, isolated_power_state):
    monkeypatch.setenv("NM_POWER_SUPERVISOR_ENABLED", "true")
    monkeypatch.setenv("NM_POWER_SUPERVISOR_LIVE_RUNTIME", "true")
    monkeypatch.setenv("NM_LIVE_RUNTIME_PORT", "8299")

    proc = MagicMock()
    proc.pid = 4242
    proc.poll.return_value = None
    popen_calls: list[list[str]] = []

    def capture_popen(*args, **kwargs):
        popen_calls.append(list(args[0]))
        return proc

    monkeypatch.setattr(ps.subprocess, "Popen", capture_popen)

    term_calls: list[MagicMock] = []

    def capture_term(p):
        term_calls.append(p)

    monkeypatch.setattr(ps, "_terminate_process", capture_term)

    iterations = {"n": 0}

    def fake_sleep(_interval):
        iterations["n"] += 1
        if iterations["n"] == 2:
            sp.set_power("off")
        if iterations["n"] >= 3:
            raise KeyboardInterrupt

    monkeypatch.setattr(ps.time, "sleep", fake_sleep)

    ps.run_supervisor_loop()

    assert len(popen_calls) == 1
    assert "8299" in popen_calls[0]
    assert proc in term_calls

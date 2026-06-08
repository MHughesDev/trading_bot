"""Desktop one-click launcher orchestration (no GUI, no real subprocesses)."""

from __future__ import annotations

from pathlib import Path

import pytest

from operator_packaging.desktop_app import launcher
from operator_packaging.desktop_app.process_supervisor import (
    DesktopProcessSupervisor,
    ServiceSpec,
    build_service_specs,
    api_health_url,
    wait_for_http,
)


class _FakeProc:
    def __init__(self, args, env=None, cwd=None, *, exit_code=None):
        self.args = args
        self.env = env
        self.cwd = cwd
        self._exit_code = exit_code
        self.terminated = False
        self.killed = False

    def poll(self):
        return self._exit_code

    def terminate(self):
        self.terminated = True
        self._exit_code = -15

    def wait(self, timeout=None):
        return self._exit_code

    def kill(self):
        self.killed = True
        self._exit_code = -9


def test_build_service_specs_default_excludes_supervisor():
    specs = build_service_specs("/usr/bin/python")
    names = [s.name for s in specs]
    # React SPA is served by FastAPI — no separate UI process.
    assert names == ["api"]
    api = next(s for s in specs if s.name == "api")
    assert "uvicorn" in api.args and "control_plane.api:app" in api.args


def test_build_service_specs_with_supervisor_and_ports():
    specs = build_service_specs(
        "py", api_port=9000, ui_port=9501, enable_supervisor=True
    )
    assert [s.name for s in specs] == ["api", "supervisor"]
    api = next(s for s in specs if s.name == "api")
    assert "9000" in api.args


def test_api_health_url():
    assert api_health_url("127.0.0.1", 8001) == "http://127.0.0.1:8001/status"


def test_supervisor_start_passes_env_cwd_and_stops_in_reverse():
    spawned: list[_FakeProc] = []

    def spawn(args, env=None, cwd=None):
        p = _FakeProc(args, env=env, cwd=cwd)
        spawned.append(p)
        return p

    specs = [ServiceSpec("a", ["x"]), ServiceSpec("b", ["y"])]
    sup = DesktopProcessSupervisor(specs, env={"K": "V"}, cwd="/repo", spawn=spawn)
    sup.start()
    assert [p.args for p in spawned] == [["x"], ["y"]]
    assert all(p.env == {"K": "V"} and p.cwd == "/repo" for p in spawned)
    assert sup.first_dead() is None
    sup.stop()
    assert all(p.terminated for p in spawned)


def test_supervisor_default_spawn_suppresses_windows_and_writes_logs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    import operator_packaging.desktop_app.process_supervisor as ps

    captured: list[dict] = []

    def fake_popen(args, **kwargs):
        captured.append({"args": args, **kwargs})
        return _FakeProc(args)

    monkeypatch.setattr(ps, "_no_window_creationflags", lambda: 0x08000000)
    monkeypatch.setattr(ps.subprocess, "Popen", fake_popen)

    log_dir = tmp_path / "logs"
    sup = ps.DesktopProcessSupervisor(
        [ServiceSpec("api", ["x"]), ServiceSpec("supervisor", ["y"])], log_dir=log_dir
    )
    sup.start()

    assert all(c["creationflags"] == 0x08000000 for c in captured)
    assert (log_dir / "api.log").exists()
    assert (log_dir / "supervisor.log").exists()
    assert all("stdout" in c and c["stderr"] is ps.subprocess.STDOUT for c in captured)
    sup.stop()


def test_supervisor_first_dead_detects_early_exit():
    def spawn(args, env=None, cwd=None):
        return _FakeProc(args, exit_code=1)

    sup = DesktopProcessSupervisor([ServiceSpec("api", ["x"])], spawn=spawn)
    sup.start()
    assert sup.first_dead() == "api"


def test_wait_for_http_returns_true_when_probe_eventually_succeeds():
    calls = {"n": 0}

    def probe(url):
        calls["n"] += 1
        return calls["n"] >= 3

    ok = wait_for_http(
        "http://x", timeout_s=10, interval_s=0, probe=probe, sleep=lambda _s: None
    )
    assert ok is True
    assert calls["n"] == 3


def test_wait_for_http_times_out():
    ticks = iter([0.0, 0.4, 0.8, 1.2, 1.6])

    ok = wait_for_http(
        "http://x",
        timeout_s=1.0,
        interval_s=0,
        probe=lambda _u: False,
        sleep=lambda _s: None,
        now=lambda: next(ticks),
    )
    assert ok is False


def test_desktop_env_forces_login_and_no_idle_timeout():
    env = launcher.desktop_env({"NM_CONTROL_PLANE_API_KEY": "secret"}, api_port=8001)
    assert env["NM_AUTH_SESSION_ENABLED"] == "true"
    assert int(env["NM_AUTH_IDLE_TIMEOUT_SECONDS"]) > 10_000_000
    # API key bypass removed so a human must sign in.
    assert "NM_CONTROL_PLANE_API_KEY" not in env
    assert env["NM_DESKTOP_URL"] == "http://127.0.0.1:8001"


def test_resolve_python_exe_prefers_venv(tmp_path: Path):
    venv_py = tmp_path / ".venv" / "bin" / "python"
    venv_py.parent.mkdir(parents=True)
    venv_py.write_text("#!/bin/sh\n")
    assert launcher.resolve_python_exe(tmp_path) == str(venv_py)


def test_resolve_python_exe_falls_back_to_sys_executable(tmp_path: Path):
    import sys

    assert launcher.resolve_python_exe(tmp_path) == sys.executable


def test_main_aborts_when_service_dies(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(launcher, "reset_sessions_for_fresh_login", lambda: 0)
    monkeypatch.setattr(launcher, "resolve_python_exe", lambda root=None: "py")

    class _DeadSup:
        def __init__(self, *a, **k):
            pass

        def start(self):
            pass

        def first_dead(self):
            return "api"

        def stop(self, **k):
            self.stopped = True

    monkeypatch.setattr(launcher, "DesktopProcessSupervisor", _DeadSup)
    monkeypatch.setattr(launcher, "wait_for_http", lambda *a, **k: False)
    opened = {"called": False}
    monkeypatch.setattr(
        launcher, "open_window", lambda *a, **k: opened.__setitem__("called", True)
    )

    rc = launcher.main()
    assert rc == 1
    assert opened["called"] is False


def test_main_opens_window_and_stops_on_close(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(launcher, "reset_sessions_for_fresh_login", lambda: 3)
    monkeypatch.setattr(launcher, "resolve_python_exe", lambda root=None: "py")

    events: list[str] = []

    class _Sup:
        def __init__(self, *a, **k):
            pass

        def start(self):
            events.append("start")

        def first_dead(self):
            return None

        def stop(self, **k):
            events.append("stop")

    monkeypatch.setattr(launcher, "DesktopProcessSupervisor", _Sup)
    monkeypatch.setattr(launcher, "wait_for_http", lambda *a, **k: True)
    monkeypatch.setattr(
        launcher, "open_window", lambda *a, **k: events.append("window")
    )

    rc = launcher.main()
    assert rc == 0
    assert events == ["start", "window", "stop"]

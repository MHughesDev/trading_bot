"""Process orchestration for the one-click desktop app (no GUI here, so it is testable).

The launcher (``launcher.py``) wires this to a native window; this module only
knows how to build the service command lines, start them, wait for the UI to be
reachable, and tear everything down. ``spawn`` and ``probe`` are injectable so the
logic can be unit-tested without real subprocesses or sockets.
"""

from __future__ import annotations

import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Any

# Defaults — local-only binds; the desktop window is the single client.
DEFAULT_API_HOST = "127.0.0.1"
DEFAULT_API_PORT = 8000
DEFAULT_UI_HOST = "127.0.0.1"
DEFAULT_UI_PORT = 8501


@dataclass(frozen=True)
class ServiceSpec:
    """One child process: a friendly ``name`` and the full ``argv`` to run."""

    name: str
    args: list[str]


def build_service_specs(
    python_exe: str,
    *,
    api_host: str = DEFAULT_API_HOST,
    api_port: int = DEFAULT_API_PORT,
    ui_host: str = DEFAULT_UI_HOST,
    ui_port: int = DEFAULT_UI_PORT,
    enable_supervisor: bool = False,
    home_path: str = "control_plane/Home.py",
) -> list[ServiceSpec]:
    """Build the API + (optional power supervisor) + Streamlit command lines.

    The power supervisor (which can start the live trading runtime) is **off by
    default** for the desktop app so "just trying it out" never touches a venue;
    enable it with ``NM_DESKTOP_START_SUPERVISOR=true``.
    """
    specs: list[ServiceSpec] = [
        ServiceSpec(
            "api",
            [
                python_exe,
                "-m",
                "uvicorn",
                "control_plane.api:app",
                "--host",
                api_host,
                "--port",
                str(int(api_port)),
            ],
        )
    ]
    if enable_supervisor:
        specs.append(ServiceSpec("supervisor", [python_exe, "-m", "app.runtime.power_supervisor"]))
    specs.append(
        ServiceSpec(
            "ui",
            [
                python_exe,
                "-m",
                "streamlit",
                "run",
                home_path,
                "--server.headless",
                "true",
                "--server.address",
                ui_host,
                "--server.port",
                str(int(ui_port)),
            ],
        )
    )
    return specs


def _no_window_creationflags() -> int:
    """``CREATE_NO_WINDOW`` on Windows (suppresses child console windows), else 0."""
    if sys.platform.startswith("win"):
        return int(getattr(subprocess, "CREATE_NO_WINDOW", 0))
    return 0


def streamlit_health_url(ui_host: str = DEFAULT_UI_HOST, ui_port: int = DEFAULT_UI_PORT) -> str:
    """Streamlit's built-in health endpoint (returns 200 once the server is up)."""
    return f"http://{ui_host}:{int(ui_port)}/_stcore/health"


def _default_probe(url: str, *, timeout: float = 1.5) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:  # noqa: S310 (local URL only)
            return 200 <= int(resp.status) < 500
    except (urllib.error.URLError, OSError, ValueError):
        return False


def wait_for_http(
    url: str,
    *,
    timeout_s: float = 60.0,
    interval_s: float = 0.5,
    probe: Callable[[str], bool] = _default_probe,
    sleep: Callable[[float], None] = time.sleep,
    now: Callable[[], float] = time.monotonic,
) -> bool:
    """Poll ``url`` until ``probe`` returns True or ``timeout_s`` elapses."""
    deadline = now() + float(timeout_s)
    while now() < deadline:
        if probe(url):
            return True
        sleep(interval_s)
    return probe(url)


class DesktopProcessSupervisor:
    """Start/stop a group of child processes and report if any died early."""

    def __init__(
        self,
        specs: list[ServiceSpec],
        *,
        env: dict[str, str] | None = None,
        cwd: str | None = None,
        spawn: Callable[..., Any] | None = None,
        log_dir: str | Path | None = None,
    ) -> None:
        self._specs = list(specs)
        self._env = env
        self._cwd = cwd
        # Default spawn suppresses child console windows on Windows and sends
        # their output to per-service log files instead of new terminals.
        self._spawn = spawn or self._windowless_spawn
        self._log_dir = Path(log_dir) if log_dir is not None else None
        self._procs: list[tuple[str, Any]] = []
        self._log_handles: list[IO[Any]] = []

    @property
    def names(self) -> list[str]:
        return [name for name, _ in self._procs]

    def _windowless_spawn(self, args: list[str], *, env=None, cwd=None, name: str = "service"):
        """Real-process spawn: no extra console window, output → a log file."""
        kwargs: dict[str, Any] = {"env": env, "cwd": cwd}
        flags = _no_window_creationflags()
        if flags:
            kwargs["creationflags"] = flags
        if self._log_dir is not None:
            self._log_dir.mkdir(parents=True, exist_ok=True)
            log_fh = open(self._log_dir / f"{name}.log", "w", encoding="utf-8", buffering=1)  # noqa: SIM115
            self._log_handles.append(log_fh)
            kwargs["stdout"] = log_fh
            kwargs["stderr"] = subprocess.STDOUT
        return subprocess.Popen(args, **kwargs)

    def start(self) -> None:
        """Launch every spec in order. Already-started supervisors are a no-op."""
        if self._procs:
            return
        for spec in self._specs:
            proc = self._call_spawn(spec)
            self._procs.append((spec.name, proc))

    def _call_spawn(self, spec: ServiceSpec) -> Any:
        """Invoke the spawn callable, passing ``name`` only if it is accepted.

        Injected test spawns use ``(args, env, cwd)``; the built-in windowless
        spawn also takes ``name`` so each child gets its own log file.
        """
        try:
            return self._spawn(spec.args, env=self._env, cwd=self._cwd, name=spec.name)
        except TypeError:
            return self._spawn(spec.args, env=self._env, cwd=self._cwd)

    def first_dead(self) -> str | None:
        """Name of the first child that has already exited, else None."""
        for name, proc in self._procs:
            poll = getattr(proc, "poll", None)
            if callable(poll) and poll() is not None:
                return name
        return None

    def stop(self, *, timeout_s: float = 8.0) -> None:
        """Terminate children (SIGTERM, then kill), youngest first."""
        for name, proc in reversed(self._procs):
            self._terminate_one(proc, timeout_s=timeout_s)
        self._procs = []
        for fh in self._log_handles:
            try:
                fh.close()
            except Exception:
                pass
        self._log_handles = []

    @staticmethod
    def _terminate_one(proc: Any, *, timeout_s: float) -> None:
        poll = getattr(proc, "poll", None)
        if callable(poll) and poll() is not None:
            return
        try:
            proc.terminate()
        except Exception:
            pass
        wait = getattr(proc, "wait", None)
        if callable(wait):
            try:
                wait(timeout=timeout_s)
                return
            except Exception:
                pass
        try:
            proc.kill()
        except Exception:
            pass

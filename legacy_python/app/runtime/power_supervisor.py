"""Watch ``data/system_power.json`` and start/stop background trading processes.

Started by ``run.bat`` (or ``python -m app.runtime.power_supervisor``) after the
control plane is up. When power is **off**, child processes are terminated so the
Kraken loop and optional live runtime stop; inference/training are already gated
via ``run_decision_tick`` / ``run_training_campaign``.

Disable with ``NM_POWER_SUPERVISOR_ENABLED=false`` (API + dashboard still run).
"""

from __future__ import annotations

import logging
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from app.runtime.system_power import is_on, sync_from_disk

logger = logging.getLogger(__name__)

_REPO_ROOT = Path(__file__).resolve().parents[2]


def _enabled() -> bool:
    return os.getenv("NM_POWER_SUPERVISOR_ENABLED", "true").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def _live_runtime_enabled() -> bool:
    return os.getenv("NM_POWER_SUPERVISOR_LIVE_RUNTIME", "true").strip().lower() in (
        "1",
        "true",
        "yes",
    )


def _poll_interval_s() -> float:
    try:
        return max(0.5, float(os.getenv("NM_POWER_SUPERVISOR_POLL_INTERVAL", "2.0")))
    except ValueError:
        return 2.0


def _live_runtime_port() -> int:
    try:
        return int(os.getenv("NM_LIVE_RUNTIME_PORT", "8208"))
    except ValueError:
        return 8208


def _terminate_process(proc: subprocess.Popen[bytes]) -> None:
    if proc.poll() is not None:
        return
    pid = proc.pid
    try:
        if sys.platform == "win32":
            subprocess.run(
                ["taskkill", "/PID", str(pid), "/T", "/F"],
                capture_output=True,
                text=True,
                timeout=60,
                check=False,
            )
        else:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=10)
    except Exception:
        logger.exception("terminate pid=%s", pid)


def _start_live_runtime(port: int) -> subprocess.Popen[bytes]:
    env = os.environ.copy()
    env["NM_LIVE_SERVICE_APP_START_LOOP"] = "true"
    args = [
        sys.executable,
        "-m",
        "uvicorn",
        "services.live_service_app:app",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
    ]
    creationflags = 0
    if sys.platform == "win32":
        creationflags = subprocess.CREATE_NEW_PROCESS_GROUP  # type: ignore[attr-defined]
    return subprocess.Popen(
        args,
        cwd=_REPO_ROOT,
        env=env,
        creationflags=creationflags,
    )


def run_supervisor_loop() -> None:
    """Block until SIGINT/SIGTERM; start/stop children based on disk power state."""
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s %(message)s")
    if not _enabled():
        logger.info("power supervisor disabled (NM_POWER_SUPERVISOR_ENABLED=false)")
        return

    port = _live_runtime_port()
    interval = _poll_interval_s()
    live_proc: subprocess.Popen[bytes] | None = None

    def _stop_all() -> None:
        nonlocal live_proc
        if live_proc is not None:
            logger.info("power OFF: stopping live runtime (pid=%s)", live_proc.pid)
            _terminate_process(live_proc)
            live_proc = None

    try:
        while True:
            sync_from_disk()
            if is_on():
                if _live_runtime_enabled():
                    if live_proc is not None and live_proc.poll() is not None:
                        logger.warning(
                            "live runtime exited (code=%s); will restart",
                            live_proc.returncode,
                        )
                        live_proc = None
                    if live_proc is None:
                        try:
                            logger.info("power ON: starting live runtime on port %s", port)
                            live_proc = _start_live_runtime(port)
                        except Exception:
                            logger.exception("failed to start live runtime")
            else:
                _stop_all()

            time.sleep(interval)
    except KeyboardInterrupt:
        logger.info("supervisor interrupted")
    finally:
        _stop_all()


def main() -> None:
    run_supervisor_loop()


if __name__ == "__main__":
    main()

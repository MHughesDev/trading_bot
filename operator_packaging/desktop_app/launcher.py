"""One-click desktop launcher for the Trading Bot.

Starts the control-plane API + Streamlit dashboard (optionally the power
supervisor), waits for the UI to come up, opens a native window on the **login
screen**, and shuts everything down when that window is closed.

Run it any of these ways (all equivalent):

    trading-bot-desktop                      # console script (after pip install)
    python -m operator_packaging.desktop_app
    python operator_packaging/desktop_app/launcher.py

Session behavior (matches "stay signed in until the app closes"):
  * existing sessions are cleared on launch  -> you always start at the login
    screen;
  * the idle lockout is suppressed while running -> you are not kicked out
    mid-session; closing the window ends the run and the next launch asks for
    login again.
"""

from __future__ import annotations

import os
import sys
import time
from collections.abc import Callable
from pathlib import Path

from operator_packaging.desktop_app.process_supervisor import (
    DEFAULT_API_HOST,
    DEFAULT_API_PORT,
    DEFAULT_UI_HOST,
    DEFAULT_UI_PORT,
    DesktopProcessSupervisor,
    build_service_specs,
    streamlit_health_url,
    wait_for_http,
)

# Effectively "never idle out" while the app window is open (~10 years).
_NO_IDLE_TIMEOUT_SECONDS = "315360000"


def repo_root() -> Path:
    """Repository root (two levels up from this file)."""
    return Path(__file__).resolve().parents[2]


def resolve_python_exe(root: Path | None = None) -> str:
    """Prefer the project ``.venv`` interpreter; fall back to the current one."""
    root = root or repo_root()
    candidates = [
        root / ".venv" / "bin" / "python",
        root / ".venv" / "Scripts" / "python.exe",
    ]
    for c in candidates:
        if c.exists():
            return str(c)
    return sys.executable


def _truthy(raw: str) -> bool:
    return raw.strip().lower() in ("1", "true", "yes", "on")


def desktop_env(
    base_env: dict[str, str] | None = None,
    *,
    ui_host: str = DEFAULT_UI_HOST,
    ui_port: int = DEFAULT_UI_PORT,
) -> dict[str, str]:
    """Environment for the child processes so the app opens on the login screen.

    Forces session login (route guard on, sessions enabled), suppresses the idle
    lockout for the lifetime of the run, and points the desktop window helper at
    the Streamlit URL. Honors anything the operator already set except where the
    desktop experience requires a specific value.
    """
    env = dict(os.environ if base_env is None else base_env)
    # Require login and keep the user signed in until the app closes.
    env["NM_STREAMLIT_ROUTE_GUARD_ENABLED"] = "true"
    env["NM_AUTH_SESSION_ENABLED"] = "true"
    env["NM_AUTH_IDLE_TIMEOUT_SECONDS"] = _NO_IDLE_TIMEOUT_SECONDS
    # An API key in the Streamlit process would bypass the login gate — drop it
    # so a human actually signs in.
    env.pop("NM_CONTROL_PLANE_API_KEY", None)
    # The desktop window points at the Streamlit UI, not the API.
    env.setdefault("NM_STREAMLIT_DESKTOP_URL", f"http://{ui_host}:{int(ui_port)}")
    env.setdefault("NM_STREAMLIT_DESKTOP_TITLE", "Trading Bot")
    return env


def reset_sessions_for_fresh_login() -> int:
    """Clear all operator sessions so the launch starts at the login screen.

    Best-effort: a missing DB or import problem must not block the launch.
    """
    try:
        from app.config.settings import load_settings
        from app.runtime import operator_sessions

        settings = load_settings()
        return operator_sessions.clear_all_sessions(settings.auth_users_db_path)
    except Exception:
        return 0


def open_window(url: str, title: str) -> None:
    """Open the native window (blocks until the window is closed)."""
    import webview  # imported lazily so non-GUI tests/CI do not need it

    webview.create_window(title, url, width=1280, height=860)
    webview.start()


def serve_in_browser_until_interrupt(
    url: str,
    *,
    is_alive: Callable[[], bool],
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Fallback when pywebview is missing: open the system browser and block.

    Keeps the stack running until the operator interrupts (Ctrl+C) or a child
    process dies. The caller's ``finally`` then stops the services.
    """
    import webbrowser

    webbrowser.open(url)
    try:
        while is_alive():
            sleep(0.5)
    except KeyboardInterrupt:
        pass


def main() -> int:
    root = repo_root()
    ui_host = os.getenv("NM_DESKTOP_UI_HOST", DEFAULT_UI_HOST)
    ui_port = int(os.getenv("NM_DESKTOP_UI_PORT", str(DEFAULT_UI_PORT)))
    api_host = os.getenv("NM_DESKTOP_API_HOST", DEFAULT_API_HOST)
    api_port = int(os.getenv("NM_DESKTOP_API_PORT", str(DEFAULT_API_PORT)))
    enable_supervisor = _truthy(os.getenv("NM_DESKTOP_START_SUPERVISOR", ""))
    startup_timeout = float(os.getenv("NM_DESKTOP_STARTUP_TIMEOUT_SECONDS", "90"))

    env = desktop_env(ui_host=ui_host, ui_port=ui_port)
    title = env.get("NM_STREAMLIT_DESKTOP_TITLE", "Trading Bot")
    ui_url = env.get("NM_STREAMLIT_DESKTOP_URL", f"http://{ui_host}:{int(ui_port)}")

    cleared = reset_sessions_for_fresh_login()
    print(f"[desktop] cleared {cleared} prior session(s); starting on the login screen.")

    specs = build_service_specs(
        resolve_python_exe(root),
        api_host=api_host,
        api_port=api_port,
        ui_host=ui_host,
        ui_port=ui_port,
        enable_supervisor=enable_supervisor,
    )
    supervisor = DesktopProcessSupervisor(
        specs, env=env, cwd=str(root), log_dir=root / "logs" / "desktop"
    )

    print("[desktop] starting services:", ", ".join(s.name for s in specs))
    print(f"[desktop] service output -> {root / 'logs' / 'desktop'} (no extra windows)")
    supervisor.start()
    try:
        print(f"[desktop] waiting for the dashboard at {ui_url} ...")
        ready = wait_for_http(
            streamlit_health_url(ui_host, ui_port),
            timeout_s=startup_timeout,
        )
        dead = supervisor.first_dead()
        if dead is not None:
            print(f"[desktop] ERROR: '{dead}' exited during startup. See output above.")
            return 1
        if not ready:
            print(f"[desktop] ERROR: dashboard did not become ready within {startup_timeout:.0f}s.")
            return 1

        print("[desktop] dashboard is up — opening the app window.")
        try:
            open_window(ui_url, title)
        except ImportError:
            print(
                "[desktop] pywebview is not installed — falling back to your browser.\n"
                '    For the dedicated app window run: pip install -e ".[dashboard]"\n'
                f"[desktop] dashboard: {ui_url}  (Ctrl+C in this window to stop)"
            )
            serve_in_browser_until_interrupt(
                ui_url, is_alive=lambda: supervisor.first_dead() is None
            )
        return 0
    finally:
        print("[desktop] window closed — stopping services.")
        supervisor.stop()


if __name__ == "__main__":
    raise SystemExit(main())

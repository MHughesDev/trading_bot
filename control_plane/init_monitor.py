"""Init pipeline monitor (FB-UX-009) — dashboard panel for running asset init jobs."""

from __future__ import annotations

from typing import Any

from control_plane.app_toasts import maybe_toast
from control_plane.asset_lifecycle_actions import init_job_is_terminal
from control_plane.streamlit_util import api_get_json


def should_emit_init_terminal_toast(prev_status: str | None, job: dict[str, Any]) -> bool:
    """True when status just became terminal (for ``st.toast`` once per completion). FB-UX-013."""
    if not prev_status:
        return False
    if prev_status.lower() in ("succeeded", "failed"):
        return False
    st_st = str(job.get("status") or "").lower()
    return init_job_is_terminal(job) and st_st in ("succeeded", "failed")


def _step_summary(steps: list[dict[str, Any]]) -> str:
    if not steps:
        return "—"
    last = steps[-1]
    name = str(last.get("step") or "?")
    stt = str(last.get("status") or "?")
    return f"{name}: {stt}"


def render_init_pipeline_monitor() -> None:
    """
    Show **symbol**, **job_id**, status, and last step; poll ``GET /assets/init/jobs/{id}`` while running.

    ``st.session_state["init_monitor_job_id"]`` is set when init starts from the Asset page.
    """
    import streamlit as st

    jid = str(st.session_state.get("init_monitor_job_id") or "").strip()
    if not jid:
        st.markdown("**Init pipeline**")
        st.caption("No active init job on this browser session. Start **Initialize** on an **Asset** page.")
        st.session_state.pop("_init_monitor_last_status", None)
        return

    try:
        job = api_get_json(f"/assets/init/jobs/{jid}")
    except Exception as e:
        st.warning(f"**Init pipeline** — could not load job `{jid}`: {e}")
        if st.button("Clear init monitor", key="init_monitor_clear_err"):
            st.session_state.pop("init_monitor_job_id", None)
            st.session_state.pop("_init_monitor_last_status", None)
            st.rerun()
        return

    sym = str(job.get("symbol") or "?")
    status = str(job.get("status") or "?")
    steps = job.get("steps") if isinstance(job.get("steps"), list) else []
    err = job.get("error")

    st.markdown("**Init pipeline**")
    c1, c2 = st.columns([3, 1])
    with c1:
        st.caption(f"**Symbol:** `{sym}` · **Job:** `{jid}` · **Status:** `{status}`")
        st.caption(f"**Last step:** {_step_summary(steps)}")
        if err:
            st.error(str(err))
    with c2:
        if st.button("Stop following", key="init_monitor_stop_follow"):
            st.session_state.pop("init_monitor_job_id", None)
            st.session_state.pop("_init_monitor_last_status", None)
            st.rerun()

    prev_st = st.session_state.get("_init_monitor_last_status")
    st_st = str(status).lower()
    if should_emit_init_terminal_toast(str(prev_st) if prev_st is not None else None, job):
        if st_st == "succeeded":
            maybe_toast(f"Init finished: {sym}", icon="✅")
        else:
            maybe_toast(f"Init failed: {sym}", icon="⚠️")
    st.session_state["_init_monitor_last_status"] = st_st

    if not init_job_is_terminal(job):
        import time

        time.sleep(0.4)
        st.rerun()

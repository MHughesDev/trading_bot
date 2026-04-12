"""Dashboard nightly scheduler panel (FB-UX-012) — ``GET /scheduler/nightly``."""

from __future__ import annotations

import json
from typing import Any

from control_plane.app_toasts import maybe_toast
from control_plane.streamlit_util import api_get_json


def _nightly_finish_message(report: Any, *, error: str | None) -> str:
    if error:
        return f"Nightly job failed: {error[:280]}"
    if not isinstance(report, dict):
        return "Nightly training finished."
    if report.get("skipped") and report.get("reason") == "no_manifest_symbols":
        return "Nightly training skipped (no initialized assets)."
    if report.get("per_asset") and isinstance(report.get("reports"), dict):
        parts: list[str] = []
        for sym, sub in report["reports"].items():
            if not isinstance(sub, dict):
                continue
            rs = sub.get("rl_skipped")
            if isinstance(rs, dict) and rs.get("reason") == "no_trade_markers_in_lookback":
                parts.append(f"{sym}: RL skipped (no trades)")
        if parts:
            return "Nightly done — " + "; ".join(parts[:6])
    return "Nightly training finished."


def _maybe_toast_scheduler_transition(prev: dict[str, Any] | None, cur: dict[str, Any]) -> None:
    """FB-UX-013: toast when a new nightly run completes (``last_run_finished_utc`` changes)."""
    if prev is None:
        return
    p_fin = prev.get("last_run_finished_utc")
    c_fin = cur.get("last_run_finished_utc")
    if not c_fin or c_fin == p_fin:
        return
    err = cur.get("last_error")
    msg = _nightly_finish_message(cur.get("last_report"), error=str(err) if err else None)
    maybe_toast(msg, icon="⚠️" if err else "✅")


def _fmt_report_brief(report: Any, *, max_chars: int = 1200) -> str:
    if report is None:
        return "—"
    try:
        s = json.dumps(report, indent=2, default=str)
    except TypeError:
        s = str(report)
    if len(s) > max_chars:
        return s[:max_chars] + "\n… (truncated)"
    return s


def render_scheduler_panel() -> None:
    """Show next/last run, interval, last error, and expandable last report JSON."""
    import streamlit as st

    st.markdown("**Nightly scheduler**")
    st.caption("In-process job while the control plane runs (**FB-AP-035**–**037**); see **`GET /scheduler/nightly`**.")
    try:
        d = api_get_json("/scheduler/nightly")
    except Exception as e:
        st.warning(f"Could not load scheduler status: {e}")
        return

    prev = st.session_state.get("_scheduler_nightly_snapshot")
    if isinstance(prev, dict):
        _maybe_toast_scheduler_transition(prev, d)
    st.session_state["_scheduler_nightly_snapshot"] = {
        "last_run_finished_utc": d.get("last_run_finished_utc"),
        "last_error": d.get("last_error"),
        "last_report": d.get("last_report"),
    }

    enabled = bool(d.get("enabled"))
    interval_s = int(d.get("interval_seconds") or 0)
    running = bool(d.get("running"))
    last_tick = d.get("last_tick_utc") or "—"
    last_fin = d.get("last_run_finished_utc") or "—"
    nxt = d.get("next_run_after_utc") or "—"
    err = d.get("last_error")

    st.caption(
        f"**Enabled:** `{enabled}` · **interval:** `{interval_s}s` · **thread alive:** `{running}` · "
        f"**per-asset forecaster:** `{d.get('nightly_per_asset_forecaster')}` · "
        f"**RL requires trade:** `{d.get('nightly_rl_requires_trade')}`"
    )
    c1, c2 = st.columns(2)
    with c1:
        st.markdown(f"**Last tick (job start)**  \n`{last_tick}`")
        st.markdown(f"**Last run finished**  \n`{last_fin}`")
    with c2:
        st.markdown(f"**Next run after (approx.)**  \n`{nxt}`")
        lb = d.get("nightly_rl_trade_lookback_days")
        st.caption(f"RL trade lookback days: `{lb}`")

    if err:
        st.error(f"Last error: {err}")
    else:
        st.success("No error recorded on last nightly run.")

    with st.expander("Last nightly report (JSON)", expanded=False):
        st.code(_fmt_report_brief(d.get("last_report")), language="json")

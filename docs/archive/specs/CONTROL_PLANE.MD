# Control plane

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## FastAPI (`control_plane/api.py`)

- **Read:** `/status`, `/routes`, `/params`, `/system/mode`, `/models`, `/metrics` (Prometheus text), `/assets/*`, `/auth/*`, `/portfolio/*`, `/pnl/*`, `/universe/*`, `/scheduler/*`, … — see `control_plane/api.py` for the full route table
- **Write:** params, lifecycle, init jobs, execution mode, flatten, etc. — **`X-API-Key`** and/or session cookie per **`require_mutate_operator`** when **`NM_CONTROL_PLANE_API_KEY`** / **`NM_AUTH_SESSION_ENABLED`** are configured
- Uses **`StateManager`**, **`ModeManager`**, per-asset registry/lifecycle helpers from `app/runtime/`

## Streamlit (`control_plane/`)

- **`Home.py`** — Dashboard (holdings, PnL, health strip, init monitor, scheduler panel)
- **`pages/`** — **`0_Login`**, **`1_Live`**, **`2_Regimes`**, **`3_Routes`**, **`4_Models`**, **`5_Logs`**, **`6_Emergency`**, **`7_Account`**, **`Asset`** (per-symbol; query `?symbol=…`)
- **`streamlit_util.py`** — API base (`NM_CONTROL_PLANE_URL`), mutate helpers with session cookie + **`X-API-Key`**, optional route guard (**`NM_STREAMLIT_ROUTE_GUARD_ENABLED`**) — details: [`../RUNBOOKS.MD`](../RUNBOOKS.MD) *Streamlit route guard*

## Run

- API: `uvicorn control_plane.api:app --host 0.0.0.0 --port 8000`
- Dashboard: `python -m streamlit run control_plane/Home.py` (see [`README.md`](../../README.md))

# Control plane

## FastAPI (`control_plane/api.py`)

- **Read:** `/status`, `/routes`, `/params`, `/system/mode`, `/models`, `/metrics` (Prometheus text)
- **Write:** `/params`, `/system/mode`, `/flatten` — optional **`X-API-Key`** when `NM_CONTROL_PLANE_API_KEY` is set
- Uses **`StateManager`**, **`ModeManager`** from `app/runtime/`

## Streamlit (`control_plane/`)

- **`Home.py`** — Multipage entry; sidebar links
- **`pages/`** — Live, Regimes, Routes, Models, Logs, Emergency
- **`streamlit_util.py`** — Shared API base URL (`NM_CONTROL_PLANE_URL`) and links to Grafana/Loki/QuestDB console (`NM_*_URL` vars)

## Run

- `python -m streamlit run control_plane/Home.py` (see README)

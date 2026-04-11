# Windows operator UI — architecture and roadmap

This document brainstorms **low-friction Windows setup** (`setup.bat` / `run.bat`), a future **desktop shell** (`.exe`), and a **unified trading dashboard** (paper/live switch, positions, P&L by timeframe). It ties to backlog ids **FB-UI-*** and **FB-DASH-*** in [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD).

---

## 1. Current state (shipped)

| Piece | Role |
|-------|------|
| **`setup.bat`** | Creates `.venv`, `pip install -e ".[dev,dashboard]"`, optional `docker compose` for `infra/docker-compose.yml`, copies `.env.example` → `.env` if missing. |
| **`run.bat`** | Starts **FastAPI** control plane (`uvicorn control_plane.api:app :8000`) and **Streamlit** (`streamlit run control_plane/Home.py`) in separate windows. |
| **Streamlit** | Existing multipage app under `control_plane/`; calls **`NM_CONTROL_PLANE_URL`** (default `http://127.0.0.1:8000`). |

**Not started:** native `.exe`, paper/live dropdown wired to runtime mode, collapsible positions pane, aggregate P&amp;L by timeframe.

---

## 2. Target UX (brainstorm)

### 2.1 Desktop executable (future)

- **Technology options**
  - **A — PyInstaller / cx_Freeze:** bundle `python.exe` + venv site-packages + launcher script that starts API + Streamlit (or embedded browser to Streamlit port). *Pros:* one file for users. *Cons:* large binary, AV false positives, signing.
  - **B — Tauri / Electron shell:** thin window loading `http://127.0.0.1:8501`. *Pros:* small exe. *Cons:* still need Python backend or remote API.
  - **C — Keep `run.bat` + default browser:** no exe; document as “supported path” for enterprise.

- **Recommendation:** Phase 1 = **`run.bat` + browser** (done). Phase 2 = **PyInstaller** wrapper that runs the same commands and opens the browser, optional. Phase 3 = dedicated desktop shell only if needed.

### 2.2 Main dashboard

- **Paper vs live:** not only a UI label — it maps to **`NM_EXECUTION_MODE`** (`paper` | `live`) and venue adapters (Alpaca vs Coinbase). **Architecture:**
  - **Option A (safest):** dropdown writes intent to a **small config file** or **operator-only API** (`POST /system/execution-profile`) that the **next process start** reads. **Never** flip live while `run_live_loop` is running without restart.
  - **Option B:** single long-lived supervisor process that **restarts** child `live_service` with new env (complex).
  - **Default recommendation:** UI shows current mode from **`GET /status`**, “Apply” sets env file + displays **“Restart live loop required”** until user restarts `app.runtime.live_service` / `live_service_app`.

- **Collapsible left panel — open positions**
  - **Data today:** Alpaca `fetch_positions` (paper); Coinbase accounts (live). Not yet exposed as a single **GET /positions** on control plane.
  - **Design:** add **`GET /portfolio/positions`** (or `/execution/positions`) aggregating adapter snapshots + symbol, qty, avg entry if available. Streamlit sidebar polls every N seconds.
  - **Per-position P&amp;L:** requires **mark price** (Kraken or venue) × qty; add **`GET /portfolio/positions/pnl`** or enrich position DTO with `unrealized_pnl_usd` when adapter provides it.

### 2.3 Aggregate P&amp;L by timeframe (hour / day / month / year / all)

- **Gap:** Realized/unrealized P&amp;L over intervals needs **trade history + cash ledger** (QuestDB fills, or Alpaca activity API), not only open positions.
  - **MVP:** show **equity curve** or **sum of closed trades** from QuestDB if `questdb_persist_decision_traces` / fills are stored; else **stub** with “connect QuestDB + backfill” message.
  - **Architecture:** `control_plane` or `data_plane` service: **`GET /pnl/summary?range=...`** returning `{ realized, unrealized, by_symbol? }`.

---

## 3. Security and operations

- **Secrets:** `.env` stays **out of git**; `setup.bat` must not echo keys. PyInstaller bundle must **not** embed production `.env`.
- **Live mode:** confirm dialog + type “LIVE” optional before enabling Coinbase path.
- **Risk:** mode switch does not bypass **`RiskEngine`** or signing.

---

## 4. Checklist cross-reference

See **§ “Operator: Windows + dashboard”** in [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) (FB-UI-*, FB-DASH-*) for tracked work. Readiness items from the earlier “ready to run” list are folded into **FB-DASH-01** (API prerequisites) and existing docs (`README`, `RUNBOOKS`, `.env.example`).

---

## 5. Related code

- `control_plane/api.py` — extend with positions/PnL when ready.
- `execution/adapters/*` — `fetch_positions`, future fills/history.
- `app/runtime/live_service.py` — live loop; mode from settings.

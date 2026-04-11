# Windows operator UI — architecture and roadmap

This document brainstorms **low-friction Windows setup** (`setup.bat` / `run.bat`), a future **desktop shell** (`.exe`), and a **unified trading dashboard** (paper/live switch, positions, P&L by timeframe). It ties to backlog ids **FB-UI-*** and **FB-DASH-*** in [`QUEUE.MD`](QUEUE.MD) (§2).

---

## 1. Current state (shipped)

| Piece | Role |
|-------|------|
| **`setup.bat`** | Creates `.venv`, `pip install -e ".[dev,dashboard]"`, optional `docker compose` for `infra/docker-compose.yml`, copies `.env.example` → `.env` if missing. Full operator checklist: [`READY_TO_RUN.MD`](READY_TO_RUN.MD). |
| **`run.bat`** | Starts **FastAPI** control plane (`:8000`), **`python -m app.runtime.power_supervisor`** (starts/stops **`services.live_service_app`** on **:8208** when global power is ON/OFF), and **Streamlit** (`control_plane/Home.py`). |
| **`packaging/windows/build_operator_bundle.bat`** | After **`setup.bat`**, installs PyInstaller and builds **`nm_operator_launcher.exe`** (see **`packaging/windows/nm_operator_launcher.spec`**). The frozen exe is a **thin launcher** only: it does **not** embed the venv or app — it finds the repo root (directory containing **`setup.bat`**) and spawns **`.venv\Scripts\python.exe`** with the same commands as **`run.bat`**. Output: **`dist/nm_operator_launcher/`** — copy that folder beside **`setup.bat`** or run from the repo root. |
| **`run_desktop_dashboard.bat`** / **`python -m operator_packaging.desktop_shell`** | Optional **pywebview** window for Streamlit (**`NM_STREAMLIT_DESKTOP_URL`**, default **`http://127.0.0.1:8501`**). Requires **`pip install -e ".[dashboard]"`**. Start **`run.bat`** first. |
| **System power** | **`GET/POST /system/power`** and sidebar on **Home** — persists `data/system_power.json`. OFF stops inference/training (code paths) and exits the Kraken loop; the supervisor terminates the live runtime process. |
| **Streamlit** | Multipage app under `control_plane/`; calls **`NM_CONTROL_PLANE_URL`** (default `http://127.0.0.1:8000`). |

**Shipped:** execution profile UI, positions, P&amp;L panels, live confirm dialog, optional **`nm_operator_launcher`**, optional **pywebview** desktop shell (§2.2). Full stack still requires **`setup.bat`** + **`.venv`**. **Not started:** Electron/Tauri-native shells beyond pywebview.

---

## 1.1 Windows distribution (PyInstaller)

**FB-UI-01-02** — practical notes for shipping **`nm_operator_launcher.exe`**:

- **Antivirus:** PyInstaller one-file/one-folder bundles are often flagged heuristically (false positives). Prefer distributing the **folder** build with a checksum manifest, or code-sign the exe if your org requires it.
- **Code signing:** Optional Authenticode signing reduces SmartScreen warnings; not automated in-repo.
- **Size:** The launcher is small (~MB); the **real** install size is still the **clone + `.venv`** from **`setup.bat`** — do not expect a single tiny portable binary for the whole trading stack.
- **Secrets:** Never bake **`.env`** or API keys into the bundle; operators keep **`.env`** beside **`setup.bat`** as today.
- **Python path:** The exe resolves the repo root by searching upward for **`setup.bat`** from the current working directory or the exe directory (when frozen). Running from the repo root after **`setup.bat`** is the supported path.

---

## 2. Target UX (brainstorm)

### 2.1 Desktop executable (future)

- **Technology options**
  - **A — PyInstaller / cx_Freeze:** bundle `python.exe` + venv site-packages + launcher script that starts API + Streamlit (or embedded browser to Streamlit port). *Pros:* one file for users. *Cons:* large binary, AV false positives, signing.
  - **B — Tauri / Electron shell:** thin window loading `http://127.0.0.1:8501`. *Pros:* small exe. *Cons:* still need Python backend or remote API.
  - **C — Keep `run.bat` + default browser:** no exe; document as “supported path” for enterprise.

- **Recommendation:** Phase 1 = **`run.bat` + browser** (done). Phase 2 = **PyInstaller** wrapper that runs the same commands and opens the browser, optional (**FB-UI-01-01** — `nm_operator_launcher`). Phase 3 = optional **pywebview** window around the same URL (**FB-UI-02-02**).

### 2.2 Desktop shell options (spike outcome — FB-UI-02-01)

| Option | Role | Pros | Cons / notes |
|--------|------|------|----------------|
| **pywebview** (Python) | Native OS webview loading `http://127.0.0.1:8501` | Same repo + venv as Streamlit; small Python dependency; **no** separate Node/Rust toolchain | Needs **WebView2** runtime on Windows (usually present on Win10/11); Linux needs GTK/Qt per [pywebview install](https://pywebview.flowrl.com/en/guide/installation.html) |
| **Electron** | Chromium shell + your HTML/JS | Mature ecosystem | Heavy runtime; second build pipeline; overkill if UI is only Streamlit in an iframe |
| **Tauri** | System webview + Rust host | Smaller than Electron | Rust toolchain; still a second project to maintain for a URL loader |

**Decision for MVP (FB-UI-02-02):** ship **`python -m operator_packaging.desktop_shell`** behind **`[dashboard]`** (adds **pywebview**). Operators run **`run.bat`** (or **`run_desktop_dashboard.bat`** after Streamlit is up) to open a **dedicated window** instead of a browser tab. Override URL with **`NM_STREAMLIT_DESKTOP_URL`** if Streamlit is not on **8501**.

**Not in scope:** bundling the shell into a standalone `.exe` (use **PyInstaller** on `streamlit_desktop_host.py` similarly to **`nm_operator_launcher`** if needed later).

### 2.3 Main dashboard

- **Paper vs live:** not only a UI label — it maps to **`NM_EXECUTION_MODE`** (`paper` | `live`) and venue adapters (Alpaca vs Coinbase). **Architecture:**
  - **Option A (safest):** dropdown writes intent to a **small config file** or **operator-only API** (`POST /system/execution-profile`) that the **next process start** reads. **Never** flip live while `run_live_loop` is running without restart.
  - **Option B:** single long-lived supervisor process that **restarts** child `live_service` with new env (complex).
  - **Default recommendation:** UI shows current mode from **`GET /status`**, “Apply” sets env file + displays **“Restart live loop required”** until user restarts `app.runtime.live_service` / `live_service_app`.

- **Collapsible left panel — open positions**
  - **Data today:** Alpaca `fetch_positions` (paper); Coinbase accounts (live). Not yet exposed as a single **GET /positions** on control plane.
  - **Design:** add **`GET /portfolio/positions`** (or `/execution/positions`) aggregating adapter snapshots + symbol, qty, avg entry if available. Streamlit sidebar polls every N seconds.
  - **Per-position P&amp;L:** requires **mark price** (Kraken or venue) × qty; add **`GET /portfolio/positions/pnl`** or enrich position DTO with `unrealized_pnl_usd` when adapter provides it.

### 2.4 Aggregate P&amp;L by timeframe (hour / day / month / year / all)

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

See **§2 — Open queue → MEDIUM → operator UI / Windows** in [`QUEUE.MD`](QUEUE.MD) (**FB-UI-01-01**, **FB-DASH-01-01**, etc.) for granular tracked work. Readiness checklist slices: **FB-DASH-01-01** (single operator page) and **FB-DASH-01-02** (cross-links from `README`, `RUNBOOKS`, `.env.example`).

---

## 5. Related code

- `control_plane/api.py` — extend with positions/PnL when ready.
- `execution/adapters/*` — `fetch_positions`, future fills/history.
- `app/runtime/live_service.py` — live loop; mode from settings.

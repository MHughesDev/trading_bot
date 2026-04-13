# Ready to run (operator checklist)

Single checklist for a **new machine** or **first-time setup**. Details and deep links live in [`README.md`](../README.md), [`RUNBOOKS.MD`](RUNBOOKS.MD), and [`.env.example`](../.env.example).

---

## 1. Repository and Python

- [ ] Clone this repo; work from the **repository root** for all commands.
- [ ] **Python ≥ 3.11** installed (`python --version` or Windows **`py -3.11`**).

---

## 2. Virtual environment and dependencies

**Unix / macOS / Linux**

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
```

**Windows (automated)**

- [ ] Run **`setup.bat`** from the repo root — creates `.venv`, runs `pip install -e ".[dev,dashboard]"`, **`docker compose pull`** then **`up -d`** for `infra/docker-compose.yml` (so image tag changes after **`git pull`** apply without extra commands), copies `.env.example` → `.env` if missing.

**Optional extras**

- [ ] Alpaca adapter tests / paper path: `pip install -e ".[alpaca]"`  
- [ ] Streamlit dashboard: `pip install -e ".[dashboard]"` (included in `setup.bat`’s `[dev,dashboard]`)

---

## 3. Environment file (`.env`)

- [ ] Copy **`.env.example`** → **`.env`** in the repo root (or let `setup.bat` do it).
- [ ] Fill **`NM_`* variables** as needed — full list and comments: [`.env.example`](../.env.example); schema: `app/config/settings.py`.
- [ ] **Never commit** `.env` (gitignored).

**Common keys**

| Goal | Variables (see `.env.example`) |
|------|--------------------------------|
| Paper trading (Alpaca) | `NM_ALPACA_API_KEY`, `NM_ALPACA_API_SECRET`, `NM_EXECUTION_MODE=paper` |
| Live Coinbase execution | `NM_COINBASE_API_KEY`, `NM_COINBASE_API_SECRET`, signing + mode per README |
| Production-like signing | `NM_RISK_SIGNING_SECRET`; avoid `NM_ALLOW_UNSIGNED_EXECUTION=true` outside dev |
| Control plane mutations | `NM_CONTROL_PLANE_API_KEY` + `X-API-Key` on POST routes |

---

## 4. Infrastructure (optional but typical)

- [ ] **Docker**: **`setup.bat`** runs pull + up for the stack; or manually: `docker compose -f infra/docker-compose.yml pull` then `up -d` — Redis, QuestDB, Qdrant, Prometheus, Grafana, Loki (ports in [`README.md`](../README.md)).
- [ ] **Microservice scaffolds** (optional): overlay [`infra/docker-compose.microservices.yml`](../infra/docker-compose.microservices.yml) — see README “Microservices dev”.

---

## 5. Preflight and health

- [ ] **`python scripts/preflight_check.py`** — exit **0** = OK for local checks; **1** = blocking issues.
- [ ] With control plane up: **`GET http://127.0.0.1:8000/status`** — review **`preflight`** and **`production_preflight`** JSON.
- [ ] Optional connectivity smoke (no orders): `pip install -e ".[alpaca]"` then **`python scripts/smoke_credentials.py`**.

---

## 6. Run the operator UI

**Windows**

- [ ] **`run.bat`** — control plane (**8000**), power supervisor (optional live runtime **8208** when power ON), Streamlit (**8501**). Set **`NM_POWER_SUPERVISOR_ENABLED=false`** if you only want API + dashboard.

**Manual (any OS)**

```bash
uvicorn control_plane.api:app --host 127.0.0.1 --port 8000
python -m streamlit run control_plane/Home.py
```

- [ ] **`NM_CONTROL_PLANE_URL`** (Streamlit) defaults to `http://127.0.0.1:8000`.
- [ ] **Legacy system power** (optional) — set **`NM_SYSTEM_POWER_LEGACY_ENABLED=true`** to use **`GET/POST /system/power`** and `data/system_power.json`; default (**FB-AP-039**) is **off** — use **per-asset Stop** for trading watch.
- [ ] **Paper vs live** — **per-asset** on the **Asset page** / **`PUT /assets/execution-mode/{symbol}`**; process default stays **`NM_EXECUTION_MODE`** in `.env`. Optional legacy **`POST /system/execution-profile`** requires **`NM_EXECUTION_PROFILE_LEGACY_API=true`** (**FB-AP-040**).

---

## 7. Live trading loop (separate from dashboard)

- [ ] **`python -m app.runtime.live_service`** — Kraken WS → decision → risk → execution (requires network + config).
- [ ] Aligns with **paper** or **live** per `NM_EXECUTION_MODE` and venue keys — **restart** after changing mode (no hot-swap). See [`RUNBOOKS.MD`](RUNBOOKS.MD).

---

## 8. Offline training

- [ ] If **`NM_SYSTEM_POWER_LEGACY_ENABLED=true`**, ensure system **power ON** in `data/system_power.json` when you want the loop to run.
- [ ] **`python -m orchestration.nightly_retrain --mode nightly`** (or `--mode initial` for longer campaign) — real Kraken OHLC; see README.

---

## Quick reference links

| Doc | Role |
|-----|------|
| [`README.md`](../README.md) | Commands, layout, CI, microservices |
| [`RUNBOOKS.MD`](RUNBOOKS.MD) | Secrets, preflight, production/network exposure, Streamlit guard, incident, QuestDB, bind address |
| [`.env.example`](../.env.example) | All **`NM_`** placeholders |
| [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) | **Queue system** — all backlog machinery files |
| [`QUEUE_STACK.csv`](QUEUE_STACK.csv) | Next tasks (`agent_task` per row) |
| [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) | Full roadmap / completed work IDs |
| [`QUEUE.MD`](QUEUE.MD) | Queue protocol + conventions |
| [`FULL_AUDIT.md`](FULL_AUDIT.md) | Full-scope audit playbook + **§8** report deliverable |
| [`reports/AUDIT_REPORT_TEMPLATE.md`](reports/AUDIT_REPORT_TEMPLATE.md) | Audit report template |
| [`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD) | Windows / future desktop UX |

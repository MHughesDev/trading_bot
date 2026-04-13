<div align="center">

# 🤖 Trading Bot

**Multi-route AI crypto trading — one codebase for research, paper trading, and controlled live execution.**

[![Python 3.11+](https://img.shields.io/badge/python-3.11+-3776AB?style=flat&logo=python&logoColor=white)](https://www.python.org/downloads/)
[![Code style: Ruff](https://img.shields.io/badge/code%20style-ruff-261230.svg?style=flat)](https://github.com/astral-sh/ruff)

</div>

---

## ✨ What this is

| | |
|:---:|---|
| 📊 | **Kraken** ingests all **market data** (REST + WebSocket). |
| 📝 | **Alpaca** is for **paper execution** only (not market data). |
| 🔐 | **Risk + signing** gate every order; live venues use **adapters** under `execution/`. |

> **Coinbase** is optional for **live order routing** when configured — never for market data ingestion.

---

## 🚀 Quick start

```bash
python -m venv .venv && source .venv/bin/activate   # Windows: .venv\Scripts\activate
pip install -e ".[dev]"
docker compose -f infra/docker-compose.yml up -d
```

**New machine checklist:** [`docs/READY_TO_RUN.MD`](docs/READY_TO_RUN.MD)

| Goal | Command |
|------|---------|
| 🖥️ Control plane API | `uvicorn control_plane.api:app --host 0.0.0.0 --port 8000` |
| 📈 Live decision loop | `python -m app.runtime.live_service` |
| 🧪 Lint & tests | `python3 -m ruff check .` · `python3 -m pytest tests/ -q` (or `ruff` if on `PATH`) |
| 🔎 Dependency audit | `bash scripts/ci_pip_audit.sh` — **release-blocking** in CI (dedicated `.audit-venv/`, not system Python) |
| 🔒 Secret scan (local) | `docker run --rm -v "$PWD:/repo" zricethezav/gitleaks:v8.21.2 detect --source /repo --redact` |
| 🛡️ Bandit (High) | `bash scripts/ci_bandit.sh` (same as CI) |

Configure secrets in **`.env`** (see [`.env.example`](.env.example)); app settings use the **`NM_`** prefix. **`app/config/default.yaml`** defaults to **`execution.mode: paper`** (Alpaca paper); set **`NM_EXECUTION_MODE=live`** only when Coinbase keys are configured. If the live loop runs with **no venue API keys** in the environment for the active mode, it **skips `submit_order`** (one startup warning) instead of spamming errors — add keys or use Streamlit per-user onboarding when session auth is enabled.

### Deployment models (FB-CONT-007)

Two supported ways to run the **same** browser-first stack (FastAPI + Streamlit + optional live loop); pick one path per environment.

| | |
|:---:|---|
| **(A) Windows convenience** | **`setup.bat`** / **`run.bat`** from the repo root: venv, `pip install -e ".[dev]"`, optional local Docker for the data plane, then API + dashboard + optional live loop — see **[`docs/WINDOWS_OPERATOR_UI.MD`](docs/WINDOWS_OPERATOR_UI.MD)**. |
| **(B) Linux OCI / Compose / cloud** | **`Dockerfile`** + **`docker compose`** (**`infra/docker-compose*.yml`**) on Linux or a cloud VM; same **`NM_*`** config via **`.env`**. Cloud VM vs Fargate sketch: **[`docs/DEPLOY_CLOUD.MD`](docs/DEPLOY_CLOUD.MD)**. Vision notes: **[`docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD)** (**BS-001**), epic **FB-CONT-P0** in **[`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD)** ([queue system](docs/QUEUE_SCHEMA.md)). |

**Data plane (Compose stack):** **QuestDB** holds canonical OHLCV and optional DB-backed traces; **Redis** is used for pub/sub and short-lived bar cache; **Qdrant** backs optional vector memory for news context — all three in **`infra/docker-compose.yml`**.

### Container image (FB-CONT-001)

The repo includes a **root `Dockerfile`**: multi-stage is not required for the default slim runtime; the image installs **`pip install -e ".[alpaca,dashboard]"`** (control plane + Streamlit + paper broker client — **not** full `torch` / `mlflow`; add a custom build stage or extra if you need GPU training inside the image).

```bash
docker build -t trading-bot:local .
docker run --rm trading-bot:local python -c "import app; import control_plane.api"
docker run --rm -p 8000:8000 trading-bot:local api
```

Entrypoint commands: **`api`** (default, uvicorn `control_plane.api:app` on **8000**), **`streamlit`** (**8501**), **`live`** (`python -m app.runtime.live_service`), **`shell`**. Use **one process per container** and Compose to run API + UI + live side by side (**FB-CONT-P0** — see **[`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD)** · [queue system](docs/QUEUE_SCHEMA.md)).

**Compose overlay (FB-CONT-002)** — data plane + app:

```bash
# From repo root; requires `.env` (copy from `.env.example`)
docker compose -f infra/docker-compose.yml -f infra/docker-compose.app.yml up -d --build
```

Publishes **8000** (API + `/metrics`), **8501** (Streamlit). Mounts **`./data`** into the app containers for manifests, SQLite sidecars, and JSONL files. Sets **`NM_QUESTDB_HOST=questdb`**, **`NM_REDIS_URL`**, **`NM_QDRANT_URL`** for in-network services. Optional live loop: add **`--profile live`**. **Backup/restore** for QuestDB / Redis / Qdrant Docker volumes: **[`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD)** (**FB-CONT-005**).

**CI (FB-CONT-003)** — GitHub Actions **`.github/workflows/ci.yml`** runs **`docker build -t trading-bot:ci .`**, a container import smoke test, **hadolint** on **`Dockerfile`**, and an informational **Trivy** filesystem scan on PRs and **`main`** pushes. Mirror locally: **`docker build -t trading-bot:local .`** (same as CI).

**TLS edge (FB-CONT-004)** — optional **Caddy** reverse proxy: merge **`infra/docker-compose.edge.yml`** after the app file. Maps **https://api.localhost:8443** and **https://ui.localhost:8443** (self-signed **`tls internal`**; add **`api.localhost`** / **`ui.localhost`** to **`hosts`**). Edit **`infra/caddy/Caddyfile`** for real DNS + automatic HTTPS on the public internet.

### Control plane security (non-localhost)

If **:8000** / **:8501** are reachable beyond a **single trusted operator on localhost**, follow **[`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD)** — **Production / network exposure (FB-AUD-001)** (API key or sessions, TLS, firewall, CORS via **`NM_CONTROL_PLANE_CORS_ALLOW_ORIGINS`**, optional **`NM_CONTROL_PLANE_RATE_LIMIT_ENABLED`**) and **Streamlit route guard (FB-AUD-002)** (`NM_STREAMLIT_ROUTE_GUARD_ENABLED`, session env alignment, optional **`NM_CONTROL_PLANE_API_KEY`** bypass for automation). Background: **[`docs/AUDIT_CODE_REVIEW.MD`](docs/AUDIT_CODE_REVIEW.MD)**. For a **full-scope** audit playbook (security, performance, data durability, ops, etc.), see **[`docs/FULL_AUDIT.md`](docs/FULL_AUDIT.md)**.

**Streamlit sign-in, sign-up, and per-user venue keys (FB-UX-015 / FB-UX-016):** Enable **`NM_AUTH_SESSION_ENABLED=true`** for password login. Optional **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=true`** redirects unauthenticated users to **`control_plane/pages/0_Login.py`**. Optional **`NM_STREAMLIT_VENUE_KEYS_REQUIRED=true`** blocks the dashboard until each user saves **Alpaca** (paper) or **Coinbase** (live) keys in-app — requires **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`** (see **`.env.example`**). Dedicated pages: **`pages/99_Sign_up.py`**, **`pages/98_Setup_API_keys.py`**. **Kraken** market data stays **deployment-only** (**`NM_*`** env); it is not collected per user. Operator detail: **[`docs/PER_ASSET_OPERATOR.MD`](docs/PER_ASSET_OPERATOR.MD)** · runbook: **[`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD)** (*Streamlit venue key onboarding*).

### Cloud deployment (FB-CONT-006)

Primary path: **single Linux VM + Docker Compose + systemd**; alternate sketch: **AWS Fargate + EFS**. Secrets via cloud secret manager → **`.env`**; networking, disk, backups, and a second-path bullet list: **[`docs/DEPLOY_CLOUD.MD`](docs/DEPLOY_CLOUD.MD)** (see also **[`docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD)**, **FB-CONT-P0**).

---

## 🪟 Windows

From the repo root: run **`setup.bat`** (venv + install + optional Docker), then **`run.bat`** (API, dashboard, optional live loop). Details: [`docs/WINDOWS_OPERATOR_UI.MD`](docs/WINDOWS_OPERATOR_UI.MD).

---

## 📖 Documentation

| Doc | What you get |
|-----|----------------|
| [`docs/READY_TO_RUN.MD`](docs/READY_TO_RUN.MD) | Environment, Docker, preflight, venues |
| [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD) | End-to-end system tour |
| **Queue system** | **[`docs/QUEUE_SCHEMA.md`](docs/QUEUE_SCHEMA.md)** — index of all backlog machinery (`QUEUE.MD`, `QUEUE_STACK.csv`, `QUEUE_ARCHIVE.MD`, automation prompt, Add-to-Queue skill, optional scripts) |
| [`docs/QUEUE.MD`](docs/QUEUE.MD) | Queue **protocol** + conventions (small; read for rules, not full backlog) |
| [`docs/QUEUE_STACK.csv`](docs/QUEUE_STACK.csv) | **Next-task stack** — **`agent_task`** per row (read first for automation) |
| [`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD) | Full open/resolved/archive tables (human history) |
| [`docs/AUDIT_CODE_REVIEW.MD`](docs/AUDIT_CODE_REVIEW.MD) | Periodic **code review audit** (correctness, security notes, candidate backlog — not the live queue) |
| [`docs/FULL_AUDIT.md`](docs/FULL_AUDIT.md) | **Full-scope** audit playbook + **§8** standalone audit report spec · skills: `draft-audit-report`, `audit-report-to-queue` |
| [`docs/reports/AUDIT_REPORT_2026-04-13_full.md`](docs/reports/AUDIT_REPORT_2026-04-13_full.md) | **Latest full audit report** (2026-04-13) — promoted to queue **FB-AUD-018**…**020** |
| [`docs/reports/AUDIT_REPORT_TEMPLATE.md`](docs/reports/AUDIT_REPORT_TEMPLATE.md) | Template for **`AUDIT_REPORT_*.md`** deliverables |
| [`docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD`](docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD) | Brainstorm: audit report → **queue stack** |
| [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD) | Agent checklist: one queue slice → validate → PR → merge (empty **HIGH** **Open** → stop per Phase 1) |
| [`docs/ADR_CANONICAL_BAR_STORAGE.MD`](docs/ADR_CANONICAL_BAR_STORAGE.MD) | Canonical bar storage (QuestDB / Parquet / Redis) |
| [`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD) | Operations & live trading checklist |
| [`docs/DEPLOY_CLOUD.MD`](docs/DEPLOY_CLOUD.MD) | Cloud VM / Fargate deployment notes (**FB-CONT-006**) |
| [`docs/ADR_MANAGED_DATA_SERVICES.MD`](docs/ADR_MANAGED_DATA_SERVICES.MD) | Managed vs self-hosted Redis / TSDB / Qdrant (**FB-CONT-008**) |
| [`docs/PER_ASSET_OPERATOR.MD`](docs/PER_ASSET_OPERATOR.MD) | Per-asset manifest API; Streamlit **Dashboard** + **`/Asset`** route (FB-AP-027) |
| [`docs/RISK_PRECEDENCE.MD`](docs/RISK_PRECEDENCE.MD) | `RiskEngine.evaluate` check order (staleness, spread, flatten, …) |
| [`docs/KRAKEN_MARKET_DATA.MD`](docs/KRAKEN_MARKET_DATA.MD) | Kraken REST/WS — single market-data source for the pipeline |

**As-built specs:** [`docs/Specs/README.MD`](docs/Specs/README.MD) · **Repo layout & CI:** see [`AGENTS.md`](AGENTS.md) for contributors and automation.

---

## 🧩 Stack at a glance

`app/` · `data_plane/` · `models/` · `decision_engine/` · `risk_engine/` · `execution/` · `backtesting/` · `control_plane/` · `observability/` · `infra/`

**Optional extras:** `pip install -e ".[alpaca]"` (paper broker) · `pip install -e ".[dashboard]"` (Streamlit) · `pip install -e ".[all]"` (MLflow / Prefect, etc.)

---

<div align="center">

*Kraken data · Alpaca paper · Shared decision path for live & replay*

</div>

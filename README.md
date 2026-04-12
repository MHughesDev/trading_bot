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
| 🧪 Lint & tests | `ruff check .` · `python3 -m pytest tests/ -q` |

Configure secrets in **`.env`** (see [`.env.example`](.env.example)); app settings use the **`NM_`** prefix.

### Container image (FB-CONT-001)

The repo includes a **root `Dockerfile`**: multi-stage is not required for the default slim runtime; the image installs **`pip install -e ".[alpaca,dashboard]"`** (control plane + Streamlit + paper broker client — **not** full `torch` / `mlflow`; add a custom build stage or extra if you need GPU training inside the image).

```bash
docker build -t trading-bot:local .
docker run --rm trading-bot:local python -c "import app; import control_plane.api"
docker run --rm -p 8000:8000 trading-bot:local api
```

Entrypoint commands: **`api`** (default, uvicorn `control_plane.api:app` on **8000**), **`streamlit`** (**8501**), **`live`** (`python -m app.runtime.live_service`), **`shell`**. Use **one process per container** and Compose to run API + UI + live side by side (**FB-CONT-P0** / **`docs/QUEUE.MD`**).

**Compose overlay (FB-CONT-002)** — data plane + app:

```bash
# From repo root; requires `.env` (copy from `.env.example`)
docker compose -f infra/docker-compose.yml -f infra/docker-compose.app.yml up -d --build
```

Publishes **8000** (API + `/metrics`), **8501** (Streamlit). Mounts **`./data`** into the app containers for manifests, SQLite sidecars, and JSONL files. Sets **`NM_QUESTDB_HOST=questdb`**, **`NM_REDIS_URL`**, **`NM_QDRANT_URL`** for in-network services. Optional live loop: add **`--profile live`**. **Backup/restore** for QuestDB / Redis / Qdrant Docker volumes: **[`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD)** (**FB-CONT-005**).

**CI (FB-CONT-003)** — GitHub Actions **`.github/workflows/ci.yml`** runs **`docker build -t trading-bot:ci .`**, a container import smoke test, **hadolint** on **`Dockerfile`**, and an informational **Trivy** filesystem scan on PRs and **`main`** pushes. Mirror locally: **`docker build -t trading-bot:local .`** (same as CI).

**TLS edge (FB-CONT-004)** — optional **Caddy** reverse proxy: merge **`infra/docker-compose.edge.yml`** after the app file. Maps **https://api.localhost:8443** and **https://ui.localhost:8443** (self-signed **`tls internal`**; add **`api.localhost`** / **`ui.localhost`** to **`hosts`**). Edit **`infra/caddy/Caddyfile`** for real DNS + automatic HTTPS on the public internet.

---

## 🪟 Windows

From the repo root: run **`setup.bat`** (venv + install + optional Docker), then **`run.bat`** (API, dashboard, optional live loop). Details: [`docs/WINDOWS_OPERATOR_UI.MD`](docs/WINDOWS_OPERATOR_UI.MD).

---

## 📖 Documentation

| Doc | What you get |
|-----|----------------|
| [`docs/READY_TO_RUN.MD`](docs/READY_TO_RUN.MD) | Environment, Docker, preflight, venues |
| [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD) | End-to-end system tour |
| [`docs/QUEUE.MD`](docs/QUEUE.MD) | Backlog, fixes, roadmap |
| [`docs/ADR_CANONICAL_BAR_STORAGE.MD`](docs/ADR_CANONICAL_BAR_STORAGE.MD) | Canonical bar storage (QuestDB / Parquet / Redis) |
| [`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD) | Operations & live trading checklist |
| [`docs/PER_ASSET_OPERATOR.MD`](docs/PER_ASSET_OPERATOR.MD) | Per-asset manifest API; Streamlit **Dashboard** + **`/Asset`** route (FB-AP-027) |

**As-built specs:** [`docs/Specs/README.MD`](docs/Specs/README.MD) · **Repo layout & CI:** see [`AGENTS.md`](AGENTS.md) for contributors and automation.

---

## 🧩 Stack at a glance

`app/` · `data_plane/` · `models/` · `decision_engine/` · `risk_engine/` · `execution/` · `backtesting/` · `control_plane/` · `observability/` · `infra/`

**Optional extras:** `pip install -e ".[alpaca]"` (paper broker) · `pip install -e ".[dashboard]"` (Streamlit) · `pip install -e ".[all]"` (MLflow / Prefect, etc.)

---

<div align="center">

*Kraken data · Alpaca paper · Shared decision path for live & replay*

</div>

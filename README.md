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
| [`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD) | Operations & live trading checklist |

**As-built specs:** [`docs/Specs/README.MD`](docs/Specs/README.MD) · **Repo layout & CI:** see [`AGENTS.md`](AGENTS.md) for contributors and automation.

---

## 🧩 Stack at a glance

`app/` · `data_plane/` · `models/` · `decision_engine/` · `risk_engine/` · `execution/` · `backtesting/` · `control_plane/` · `observability/` · `infra/`

**Optional extras:** `pip install -e ".[alpaca]"` (paper broker) · `pip install -e ".[dashboard]"` (Streamlit) · `pip install -e ".[all]"` (MLflow / Prefect, etc.)

---

<div align="center">

*Kraken data · Alpaca paper · Shared decision path for live & replay*

</div>

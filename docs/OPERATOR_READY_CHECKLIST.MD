# Operator ready-to-run checklist

Single place to verify environment, config, and preflight before paper or live operation. Cross-linked from [`README.md`](../README.md), [`RUNBOOKS.MD`](RUNBOOKS.MD), and [`.env.example`](../.env.example). Tracked as **FB-DASH-01-01** / **FB-DASH-01-02** in [`QUEUE.MD`](QUEUE.MD).

---

## 1. Python environment

- [ ] Python **3.11+** available (`python --version`).
- [ ] Virtual environment created and activated (see Quick start in [`README.md`](../README.md)).
- [ ] Dependencies installed: `pip install -e ".[dev]"` (add `[alpaca]` for paper execution, `[dashboard]` for Streamlit).

## 2. Configuration (`.env`)

- [ ] Copy [`.env.example`](../.env.example) → `.env` in the repo root if you do not have one yet.
- [ ] Fill **`NM_`-prefixed** variables only (see `app/config/settings.py`). Never commit real keys.

## 3. Execution mode vs venue keys

| Goal | Set | Venue credentials |
|------|-----|---------------------|
| **Paper** (default) | `NM_EXECUTION_MODE=paper` (or omit) | **Alpaca:** `NM_ALPACA_API_KEY`, `NM_ALPACA_API_SECRET` |
| **Live** (Coinbase execution) | `NM_EXECUTION_MODE=live` and live adapter in config | **Coinbase CDP:** `NM_COINBASE_API_KEY`, `NM_COINBASE_API_SECRET` |

Kraken is used for **market data** only (no keys required for public REST/WS). Coinbase keys are for **orders**, not quotes.

## 4. Optional local stack (Docker)

- [ ] For QuestDB / Redis / Qdrant / observability: `docker compose -f infra/docker-compose.yml up -d` (ports in [`README.md`](../README.md)).

## 5. Preflight

- [ ] Run: `python scripts/preflight_check.py` — exits **1** if blocking issues (e.g. live mode without signing secret).
- [ ] Or with the control plane up: **`GET /status`** — inspect **`preflight`** and **`production_preflight`**.

## 6. Control plane and dashboard (optional)

- [ ] API: `uvicorn control_plane.api:app --host 0.0.0.0 --port 8000` (or your `NM_CONTROL_PLANE_*` settings).
- [ ] Dashboard: `python -m streamlit run control_plane/Home.py` (after `pip install -e ".[dashboard]"`).

## 7. After changing execution mode or adapters

Changing **`NM_EXECUTION_MODE`** or venue-related env requires **restarting** processes that load settings (`live_service`, `power_supervisor` / `live_service_app`, execution gateway). See **Restart after config change** in [`RUNBOOKS.MD`](RUNBOOKS.MD).

---

## Windows quick path

From the repo root: **`setup.bat`** then **`run.bat`** — see [`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD).

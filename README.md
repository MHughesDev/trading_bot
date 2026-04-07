# NautilusMonster V3

Multi-route AI crypto trading stack: **Coinbase** for all market data; **Alpaca** for paper execution only; typed contracts and execution adapters.

## Quick start

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
docker compose -f infra/docker-compose.yml up -d
```

Smoke-test Coinbase public WebSocket (no API keys for public channels):

```bash
python -m data_plane.ingest.coinbase_ws
```

Configure secrets via `.env` (prefix `NM_` for app settings). **Never use Alpaca for market data.**

## Control plane (FastAPI)

```bash
uvicorn control_plane.api:app --host 0.0.0.0 --port 8000
```

Endpoints: `/status`, `/routes`, `/params`, `/system/mode`, `/flatten`, `/models`, `/metrics` (Prometheus).

## Optional extras

- Paper execution (Alpaca): `pip install -e ".[alpaca]"`
- Dashboard: `pip install -e ".[dashboard]"` then `streamlit run control_plane/dashboard.py`
- MLflow/Prefect: `pip install -e ".[all]"` (see `orchestration/`)

## Layout

See Master Spec V3: `app/` (runtime, contracts, config), `data_plane/`, `models/`, `decision_engine/`, `risk_engine/`, `execution/`, `backtesting/`, `control_plane/`, `observability/`, `infra/`.

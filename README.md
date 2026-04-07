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

**Production:** set `NM_RISK_SIGNING_SECRET` so only `RiskEngine`-signed `OrderIntent`s reach venues; optional `NM_CONTROL_PLANE_API_KEY` for mutating control-plane routes. For local dev without signing, `NM_ALLOW_UNSIGNED_EXECUTION=true` (not for production).

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

## Production hardening (Master Spec checklist)

Track remaining work toward full spec compliance in [`docs/PRODUCTION_HARDENING.md`](docs/PRODUCTION_HARDENING.md) (checkboxes, no external tracker required).

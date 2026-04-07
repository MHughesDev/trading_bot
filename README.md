# NautilusMonster V3

NautilusMonster V3 is a modular AI-driven crypto trading runtime using:

- **Coinbase** as the only market data source
- **Coinbase** (live) and **Alpaca** (paper) execution adapters
- Shared decision/risk logic across live and backtest modes
- Typed contracts + structured decision traces for auditability

## Architecture

```text
Coinbase Market Data
        ↓
Feature Pipeline (Polars)
        ↓
Regime Model (HMM)
        ↓
Memory Retrieval (Qdrant)
        ↓
Forecast Model (TFT proxy)
        ↓
Route Selector
        ↓
Action Generator
        ↓
Risk Engine
        ↓
Execution Router
        ↓
[ Alpaca (paper) | Coinbase (live) ]
```

## Repository layout

```text
app/            runtime + typed contracts + config
data_plane/     ingest + bars + features + memory + storage
models/         regime + forecast + route selector + registry
decision_engine route/action generation
risk_engine/    hard constraints + mode-aware gating
execution/      adapter abstraction + routing
backtesting/    replay + execution simulator + portfolio tracker
control_plane/  FastAPI + Streamlit dashboard
orchestration/  nightly retrain flow
observability/  JSON logging + Prometheus metrics
infra/          docker compose + monitoring config
tests/          unit tests
```

## Quickstart

### 1) Install

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

### 2) Start infrastructure

```bash
docker compose -f infra/docker-compose.yml up -d
```

### 3) Run API

```bash
uvicorn control_plane.api:app --host 0.0.0.0 --port 8000
```

### 4) Run dashboard

```bash
streamlit run control_plane/dashboard.py --server.port 8501 --server.address 0.0.0.0
```

## Key API endpoints

- `GET /status`
- `GET /routes`
- `GET /params`
- `POST /system/mode`
- `POST /flatten`
- `GET /models`
- `GET /traces`
- `GET /metrics`

## Modes

System modes:

- `RUNNING`
- `PAUSE_NEW_ENTRIES`
- `REDUCE_ONLY`
- `FLATTEN_ALL`
- `MAINTENANCE`

Execution modes:

- `paper` -> Alpaca paper adapter
- `live` -> Coinbase adapter

## Notes

- No auto model promotion is performed.
- Risk engine decisions cannot be bypassed in the runtime pipeline.
- All actions are emitted as structured traces for audit.
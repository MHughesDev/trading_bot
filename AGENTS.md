# AGENTS.md

## Cursor Cloud specific instructions

### Overview

NautilusMonster V3 is an AI-driven cryptocurrency trading platform with a multi-model pipeline: TFT forecasting, Gaussian HMM regime detection, deterministic route selection, and risk-gated execution. It targets Coinbase (live) and Alpaca (paper) via adapters. The control plane is FastAPI + Streamlit.

### Dependencies

- **Python 3.12+** (pre-installed in the VM)
- Install via `pip install -e ".[dev,alpaca,dashboard,orchestration]"` from the repo root
- **Docker** is required for infrastructure services (QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki)

### Infrastructure services

Start all infrastructure with:
```
sudo docker compose -f infra/docker-compose.yml up -d
```

| Service | Port | Purpose |
|---|---|---|
| Redis | 6379 | State store, bar TTL cache |
| QuestDB | 9000 (HTTP), 8812 (PG), 9009 (ILP) | Time-series storage |
| Qdrant | 6333 (HTTP), 6334 (gRPC) | Vector memory for news/sentiment |
| Prometheus | 9090 | Metrics collection |
| Grafana | 3000 | Dashboards (admin/admin) |
| Loki | 3100 | Log aggregation |

### Running the application

**FastAPI Control Plane:**
```
uvicorn control_plane.api:app --host 0.0.0.0 --port 8000
```

**Streamlit Dashboard:**
```
python3 -m streamlit run control_plane/Home.py --server.port 8501 --server.headless true
```

### Linting

```
ruff check .
```
Config is in `pyproject.toml` (line-length=100, target py312).

### Testing

```
pytest tests/ -v
```
22 tests covering contracts, risk engine, route selector, execution router, replay parity, and spec compliance. Config: `asyncio_mode = "auto"` in `pyproject.toml`.

### Key caveats

- `streamlit` must be invoked as `python3 -m streamlit` rather than bare `streamlit` — the latter may not be on PATH.
- `pytz` is a transitive dependency of `alpaca-py` that may not auto-install; `pip install -e ".[dev,alpaca,dashboard,orchestration]"` handles this.
- Docker daemon in the VM needs `fuse-overlayfs` storage driver and `iptables-legacy` (see system-level setup).
- The `.env` file is gitignored. For live trading, set `API_KEY` and `API_SECRET` (Alpaca) plus any Coinbase keys as environment secrets.
- `NM_CONTROL_PLANE_API_KEY` protects mutating API endpoints; unset in dev means open access.
- `NM_RISK_SIGNING_SECRET` enables order intent signing in production; unset in dev means unsigned execution is allowed.

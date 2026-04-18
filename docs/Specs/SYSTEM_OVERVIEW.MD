# System overview (as implemented)

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Product boundaries

- **Market data:** **Kraken** only (WebSocket + REST under `data_plane/ingest/kraken_*`). **Alpaca** must not be used for market-data clients in the main package tree — `scripts/ci_spec_compliance.sh` fails on `alpaca.data` imports outside the paper execution adapter (`execution/adapters/alpaca_paper.py`). See [`../KRAKEN_MARKET_DATA.MD`](../KRAKEN_MARKET_DATA.MD).
- **Execution:** Config-driven — **paper** → Alpaca paper adapter; **live** → Coinbase Advanced Trade (CDP JWT) adapter. Shared **decision + risk** path for live and replay.
- **Decision path:** master pipeline — **`ForecastPacket`** (`forecaster_model`, xLSTM stack) + **`PolicySystem`** (`policy_model`) → **`ActionProposal`** ([`MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD), [`MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD)).
- **Risk:** `OrderIntent` execution is gated (HMAC signing, `execution/intent_gate.py`) per settings — not bypassed in production paths.

## Top-level packages (Python)

| Package | Role |
|---------|------|
| `app/` | Config (`AppSettings`, `default.yaml`), typed contracts, runtime (`live_service` and helpers) |
| `data_plane/` | Kraken ingest, rolling bars, Polars features, Qdrant memory, QuestDB/Redis storage |
| `decision_engine/` | `run_decision_tick` (shared with backtest), `DecisionPipeline` (master forecaster + policy), audit |
| `risk_engine/` | `RiskEngine`, order signing |
| `execution/` | `ExecutionService`, router, Alpaca paper + Coinbase live adapters, CDP HTTP client |
| `backtesting/` | Replay and portfolio simulation vs same decision step |
| `control_plane/` | FastAPI `api.py`, Streamlit multipage app |
| `models/` | Optional MLflow helpers + **filesystem active model set** (`models/registry/active_model_set.json`, FB-SPEC-06) |
| `observability/` | Prometheus metrics, structured logging |
| `orchestration/` | Nightly retrain stub, optional Prefect/MLflow |
| `infra/` | `docker-compose.yml`, Prometheus/Grafana/Promtail/Loki configs |
| `forecaster_model/` | Human-spec forecaster package (stub + features; full neural stack backlog) |
| `policy_model/` | Human-spec forecast-conditioned policy (heuristic + gate + planner stubs) |
| `tests/` | Pytest suite |
| `scripts/` | CI guards, smoke scripts |
| `legacy/cryptobot/` | Frozen snapshot — not part of the V3 pipeline |

## Main entry points (operator-facing)

- **Live loop:** `python -m app.runtime.live_service`
- **Control plane API:** `uvicorn control_plane.api:app --host 0.0.0.0 --port 8000`
- **Dashboard:** `python -m streamlit run control_plane/Home.py`
- **Local stack:** `docker compose -f infra/docker-compose.yml up -d`

## Runtime

- Python **≥ 3.11** (`pyproject.toml`).

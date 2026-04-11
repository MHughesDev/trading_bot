# System overview (as implemented)

## Product boundaries

- **Market data:** Coinbase only (WebSocket + REST). No Alpaca (or other venues) for market data in the main tree; CI enforces this (`scripts/ci_spec_compliance.sh`).
- **Execution:** Config-driven — **paper** → Alpaca paper adapter; **live** → Coinbase Advanced Trade (CDP JWT) adapter. Shared **decision + risk** path for live and replay.
- **Decision path:** master pipeline — **`ForecastPacket`** (`forecaster_model`, xLSTM stack) + **`PolicySystem`** (`policy_model`) → **`ActionProposal`** ([`MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD), [`MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD)).
- **Risk:** `OrderIntent` execution is gated (HMAC signing, `execution/intent_gate.py`) per settings — not bypassed in production paths.

## Top-level packages (Python)

| Package | Role |
|---------|------|
| `app/` | Config (`AppSettings`, `default.yaml`), typed contracts, runtime (`live_service` and helpers) |
| `data_plane/` | Coinbase ingest, rolling bars, Polars features, Qdrant memory, QuestDB/Redis storage |
| `decision_engine/` | `run_decision_tick` (shared with backtest), `DecisionPipeline` (master forecaster + policy), audit |
| `risk_engine/` | `RiskEngine`, order signing |
| `execution/` | `ExecutionService`, router, Alpaca paper + Coinbase live adapters, CDP HTTP client |
| `backtesting/` | Replay and portfolio simulation vs same decision step |
| `control_plane/` | FastAPI `api.py`, Streamlit multipage app |
| `models/` | Optional MLflow registry helpers (`models/registry/`) |
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

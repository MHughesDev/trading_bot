# NautilusMonster V3

Multi-route AI crypto trading stack: **Kraken** for all market data (REST + WebSocket); **Alpaca** for paper execution only; typed contracts and execution adapters. Live execution may still use **Coinbase** when `execution.live_adapter: coinbase` (orders only — not market data).

## Quick start

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
docker compose -f infra/docker-compose.yml up -d
```

Market data uses **Kraken** public APIs (no keys for read). Bar buckets default to **1 second** (`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`).

**Live decision loop (paper or live execution per config):** connects Kraken WS → rolling bars → `run_decision_tick` → optional QuestDB traces → venue submit. Run as a module:

```bash
python -m app.runtime.live_service
```

For **Alpaca paper**, set `NM_ALPACA_API_KEY` / `NM_ALPACA_API_SECRET`. To align in-memory positions with the broker periodically (recommended), set `NM_POSITION_RECONCILE_ENABLED=true` (see `execution.position_reconcile_*` in `app/config/default.yaml`).

Configure secrets via `.env` (prefix `NM_` for app settings — see [`.env.example`](.env.example)). **Never use Alpaca for market data.**

**Decision pipeline (only path):** master spec — **`forecaster_model`** → **`ForecastPacket`** → **`PolicySystem`** → risk / execution (see [`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](docs/Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) and [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD)). **Forecaster quantiles:** optional **`NM_MODELS_FORECASTER_TORCH_PATH`** (`forecaster_torch.pt` from `train_torch_forecaster_distill`) else **`NM_MODELS_FORECASTER_WEIGHTS_PATH`** (NPZ) else NumPy RNG. Policy: **`NM_MODELS_POLICY_MLP_PATH`** (NPZ) else heuristic. Logs `decision pipeline serving mode:` (`pytorch_mlp` / `npz_weights` / `numpy_rng`). Optional: `NM_MODELS_FORECASTER_CHECKPOINT_ID`, `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH`, **`NM_MODELS_TORCH_DEVICE`**. Train PyTorch distill: `pip install -e ".[models_torch]"` then `forecaster_model.training.train_torch_forecaster`. Historical note: [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](docs/MIGRATION_TO_SPEC_PIPELINE.MD).

**Production:** set `NM_RISK_SIGNING_SECRET` so only `RiskEngine`-signed `OrderIntent`s reach venues; optional `NM_CONTROL_PLANE_API_KEY` for mutating control-plane routes. For local dev without signing, `NM_ALLOW_UNSIGNED_EXECUTION=true` (not for production).

**Preflight (live/paper):** `python scripts/preflight_check.py` (exit 1 if blocking issues) or `GET /status` on the control plane — includes `preflight` JSON (**IL-105** / **FB-SPEC-08**).

**Verify API connectivity (no orders placed):** `pip install -e ".[alpaca]"` and `python scripts/smoke_credentials.py` (Kraken public OHLC + optional Alpaca). `NM_COINBASE_*` only matters for **live Coinbase execution**, not data.

**Offline training (real candles only):** fetches historical OHLC from **Kraken** (`NM_TRAINING_DATA_GRANULARITY_SECONDS`, default 60s); sub-minute or non-standard sizes use **Trades** aggregation (slow for long lookbacks). Walk-forward splits, quantile forecaster fit, then heuristic policy evaluation (PPO/SAC are backlog). Run:

```bash
python -m orchestration.nightly_retrain --mode nightly
# or full initial campaign (longer lookback, more jobs):
python -m orchestration.nightly_retrain --mode initial --lookback-days 180
```

Outputs under `NM_TRAINING_ARTIFACT_DIR` (default `models/artifacts_training/`): `bars.parquet`, `forecaster_quantile_real.joblib`, `training_report.json`. Specs: [`docs/Human Provided Specs/NIGHTLY_TRAINING_AND_REFRESH_SPEC.MD`](docs/Human%20Provided%20Specs/NIGHTLY_TRAINING_AND_REFRESH_SPEC.MD), [`docs/Human Provided Specs/INITIAL_OFFLINE_TRAINING_CAMPAIGN_SPEC.MD`](docs/Human%20Provided%20Specs/INITIAL_OFFLINE_TRAINING_CAMPAIGN_SPEC.MD).

## Control plane (FastAPI)

```bash
uvicorn control_plane.api:app --host 0.0.0.0 --port 8000
```

Endpoints: `/status`, `/routes`, `/params`, `/system/mode`, `/flatten`, `/models`, `/metrics` (Prometheus).

## Service scaffolds (Phase 1)

Monorepo microservice entrypoints now exist under `services/` with baseline health endpoints (`/healthz`, `/readyz`, `/status`). Run any scaffold service, for example:

```bash
uvicorn services.market_data_service.main:app --host 0.0.0.0 --port 8101
```

Shared event envelope/topic contracts are under `shared/messaging/`, along with a Phase 2 in-memory bus (`InMemoryMessageBus`) and a Redis Streams adapter baseline (publish + explicit poll).

A local Phase 3 handoff wiring helper is available at `services/pipeline_handoff.py` for in-memory topic flow (`features -> decision -> risk -> execution`) during incremental extraction.

A milestone combined service is available at `services/decision_risk_service/main.py` with `POST /simulate` to smoke-test decision→risk→execution handoff.

A runtime bridge helper is available at `services/runtime_bridge.py` to route feature payloads through the microservice handoff path for incremental runtime integration.

Set `NM_MICROSERVICES_RUNTIME_BRIDGE_ENABLED=true` to enable runtime handoff publishing from the live loop.

- **Shadow (default):** `NM_MICROSERVICES_EXECUTION_GATEWAY_MODE=in_process` — decision→risk→execution handlers run inside the live process (duplicate of real execution unless you disable submit).
- **External gateway:** `NM_MICROSERVICES_EXECUTION_GATEWAY_MODE=external` with `NM_MESSAGING_BACKEND=redis_streams` and `NM_REDIS_URL` — live publishes the handoff chain to Redis and **skips** in-process `submit_order`; run the execution gateway service separately to consume `risk.intent.accepted.v1`.

Execution gateway service exposes `POST /ingest/risk-accepted`, `GET /events/recent`, and `GET /messaging` (backend hint). With Redis, it runs a background poll loop on `risk.intent.accepted.v1`.

**Execution path:** with `NM_EXECUTION_GATEWAY_SUBMIT=true` (default), the gateway verifies the envelope and calls **`ExecutionService.submit_order`**. Set **`NM_EXECUTION_GATEWAY_SUBMIT=false`** for scaffold-only ack/fill (used by default in unit tests). **`NM_EXECUTION_ADAPTER=stub`** selects the in-process stub adapter (no venue); omit for normal paper/live adapters.
Risk service exposes `POST /ingest/decision-proposal` and `GET /events/recent` for proposal gating smoke tests.
Decision service exposes `POST /ingest/features-row` and `GET /events/recent` for feature-to-proposal smoke tests.

## Optional extras

- Paper execution (Alpaca): `pip install -e ".[alpaca]"`
- Dashboard: `pip install -e ".[dashboard]"` then `streamlit run control_plane/Home.py`
- MLflow/Prefect: `pip install -e ".[all]"` (see `orchestration/`)

## Layout

`app/` (runtime, contracts, config), `data_plane/`, `models/`, `decision_engine/`, `risk_engine/`, `execution/`, `backtesting/`, `control_plane/`, `observability/`, `infra/`.

## Documentation map

- **As-built specs (code-aligned):** [`docs/Specs/README.MD`](docs/Specs/README.MD) — topic specs that mirror the current codebase.
- **End-to-end walkthrough (live, paper, live venue, backtest):** [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD) — uses default **`spec_policy`** pipeline unless noted.
- **Human-provided intent:** [`docs/Human Provided Specs/README.MD`](docs/Human%20Provided%20Specs/README.MD) — includes **forecaster** and **policy** architecture specs; reconcile against `docs/Specs/` and the repo to drive [`docs/FEATURES_BACKLOG.MD`](docs/FEATURES_BACKLOG.MD) / [`docs/ISSUE_LOG.MD`](docs/ISSUE_LOG.MD).
- **Backlog & issues, runbooks, deep dives:** other files under [`docs/`](docs/).
- **Microservice migration blueprint (monorepo-first):** [`docs/MICROSERVICES_SPLIT_PLAN.MD`](docs/MICROSERVICES_SPLIT_PLAN.MD).

## Backtesting (simulated fees / slippage)

`backtesting.replay_decisions` supports optional portfolio accounting: set `track_portfolio=True` to apply `slippage_bps`, `fee_bps`, optional `slippage_noise_bps` with `rng_seed` for reproducibility, and `initial_cash_usd` from `app/config/default.yaml` under `backtesting:` (or `NM_BACKTESTING_*`). Default `track_portfolio=False` keeps prior behavior (position qty only). With `track_portfolio`, optional **`enforce_solvency`** (default true) skips buys that would drive simulated cash negative. **`replay_multi_asset_decisions`** runs multiple symbols on one portfolio timeline (see [`docs/BACKTESTING_SIMULATOR.MD`](docs/BACKTESTING_SIMULATOR.MD)). Semantics: same doc.

## CI

GitHub Actions (`.github/workflows/ci.yml`): **ruff**, **pytest**, `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`. Optional integration job against Redis / QuestDB / Qdrant is **manual** (`workflow_dispatch`) or run locally with `NM_INTEGRATION_SERVICES=1` after `docker compose -f infra/docker-compose.yml up -d`. That job also runs **`tests/test_integration_microservices_redis.py`** (Redis + subprocess `uvicorn` execution gateway + stub submit).

**Microservices dev (optional):** `docker compose -f infra/docker-compose.yml -f infra/docker-compose.microservices.yml up -d redis execution_gateway` starts Redis and a **stub** execution gateway on port **8202** (first start installs deps in the container).

## Operations

Runbooks (secrets, incident, flatten, QuestDB backup): [`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD).

## Roadmap & logs

**Backlog (features, hardening gates, platform):** [`docs/FEATURES_BACKLOG.MD`](docs/FEATURES_BACKLOG.MD). **Issues to fix (existing code):** [`docs/ISSUE_LOG.MD`](docs/ISSUE_LOG.MD). **Reference:** [`docs/RISK_PRECEDENCE.MD`](docs/RISK_PRECEDENCE.MD) (risk order).

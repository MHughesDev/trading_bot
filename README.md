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

**Live decision loop (paper or live execution per config):** connects Coinbase WS → rolling bars → `run_decision_tick` → optional QuestDB traces → venue submit. Run as a module:

```bash
python -m app.runtime.live_service
```

For **Alpaca paper**, set `NM_ALPACA_API_KEY` / `NM_ALPACA_API_SECRET`. To align in-memory positions with the broker periodically (recommended), set `NM_POSITION_RECONCILE_ENABLED=true` (see `execution.position_reconcile_*` in `app/config/default.yaml`).

Configure secrets via `.env` (prefix `NM_` for app settings — see [`.env.example`](.env.example)). **Never use Alpaca for market data.**

**Decision pipeline (only path):** master spec — **`forecaster_model`** (xLSTM stack) → **`ForecastPacket`** → **`PolicySystem`** → risk / execution (see [`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](docs/Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) and [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD)). At runtime this uses the **NumPy reference forecaster** and **heuristic policy** until trained weights load (**FB-SPEC-02**); startup logs `decision pipeline serving mode:`. Optional: `NM_MODELS_FORECASTER_CHECKPOINT_ID` (lineage label only), `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` (conformal JSON). Historical note: [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](docs/MIGRATION_TO_SPEC_PIPELINE.MD).

**Production:** set `NM_RISK_SIGNING_SECRET` so only `RiskEngine`-signed `OrderIntent`s reach venues; optional `NM_CONTROL_PLANE_API_KEY` for mutating control-plane routes. For local dev without signing, `NM_ALLOW_UNSIGNED_EXECUTION=true` (not for production).

**Verify API connectivity (no orders placed):** put keys in `.env` (`NM_ALPACA_*`, optional `NM_COINBASE_*`), then `pip install -e ".[alpaca]"` and `python scripts/smoke_credentials.py`. Coinbase Advanced Trade REST may require JWT for some routes; the script falls back to the public Exchange ticker when unauthenticated candle calls fail.

**Offline training (real candles only):** fetches historical 1m OHLCV from Coinbase public REST, walk-forward splits, quantile forecaster fit, then heuristic policy evaluation on the same real path (PPO/SAC are backlog). Run:

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

## Backtesting (simulated fees / slippage)

`backtesting.replay_decisions` supports optional portfolio accounting: set `track_portfolio=True` to apply `slippage_bps`, `fee_bps`, optional `slippage_noise_bps` with `rng_seed` for reproducibility, and `initial_cash_usd` from `app/config/default.yaml` under `backtesting:` (or `NM_BACKTESTING_*`). Default `track_portfolio=False` keeps prior behavior (position qty only). With `track_portfolio`, optional **`enforce_solvency`** (default true) skips buys that would drive simulated cash negative. **`replay_multi_asset_decisions`** runs multiple symbols on one portfolio timeline (see [`docs/BACKTESTING_SIMULATOR.MD`](docs/BACKTESTING_SIMULATOR.MD)). Semantics: same doc.

## CI

GitHub Actions (`.github/workflows/ci.yml`): **ruff**, **pytest**, `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`. Optional integration job against Redis / QuestDB / Qdrant is **manual** (`workflow_dispatch`) or run locally with `NM_INTEGRATION_SERVICES=1` after `docker compose -f infra/docker-compose.yml up -d`.

## Operations

Runbooks (secrets, incident, flatten, QuestDB backup): [`docs/RUNBOOKS.MD`](docs/RUNBOOKS.MD).

## Roadmap & logs

**Backlog (features, hardening gates, platform):** [`docs/FEATURES_BACKLOG.MD`](docs/FEATURES_BACKLOG.MD). **Issues to fix (existing code):** [`docs/ISSUE_LOG.MD`](docs/ISSUE_LOG.MD). **Reference:** [`docs/RISK_PRECEDENCE.MD`](docs/RISK_PRECEDENCE.MD) (risk order).

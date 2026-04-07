# GitHub issues — Master Spec V3 (batch)

The Cursor/GitHub integration token cannot create issues from CI. Use one of:

1. **Run locally:** `bash scripts/create_github_issues.sh` (requires `gh auth login` with `repo` scope).
2. **Manual:** copy each **Title** / **Body** block below into **New issue** on `MHughesDev/trading_bot`.

Suggested label: `enhancement` (create in repo if missing).

Optional **milestone:** `NautilusMonster V3 — Spec Complete`

---

## Epic (create first; link others to it)

### Title
`Epic: Master Spec V3 — remaining work`

### Body
```markdown
Parent tracker for NautilusMonster V3 spec compliance. Close when all child issues are done and `docs/PRODUCTION_HARDENING.md` is fully checked.

See also: `docs/PRODUCTION_HARDENING.md`
```

---

## Issue 1

### Title
`Data: Wire Coinbase WS feed health to risk (stale data)`

### Body
```markdown
## Goal
Align live feed staleness with `NM_RISK_STALE_DATA_SECONDS` using `CoinbaseWebSocketClient.last_message_at` / gaps, not only synthetic timestamps in `live_service`.

## Acceptance
- [ ] Risk blocks or mode reflects stale/disconnected WS before orders
- [ ] Metric or log when feed exceeds threshold

## Refs
`data_plane/ingest/coinbase_ws.py`, `risk_engine/engine.py`, `app/runtime/live_service.py`
```

---

## Issue 2

### Title
`Data: Harden Coinbase REST (rate limits, errors, candles for V1 symbols)`

### Body
```markdown
## Goal
Production-ready REST client for candles/metadata: retries, rate limits, pagination, clear errors.

## Acceptance
- [ ] BTC-USD, ETH-USD, SOL-USD candle fetch validated against live or recorded fixtures
- [ ] Documented granularity mapping for bar pipeline

## Refs
`data_plane/ingest/coinbase_rest.py`
```

---

## Issue 3

### Title
`Data: Normalizer tests from recorded Coinbase WS payloads`

### Body
```markdown
## Goal
Contract tests using real JSON fixtures; unknown messages increment metrics, never corrupt state.

## Acceptance
- [ ] Fixture files under `tests/fixtures/coinbase_ws/`
- [ ] Tests cover ticker, trades, L2, candles paths in `normalizers.py`

## Refs
`data_plane/ingest/normalizers.py`
```

---

## Issue 4

### Title
`Data: Product metadata cache (tick size, min size, status)`

### Body
```markdown
## Goal
Cache Coinbase product metadata for sizing and filters.

## Acceptance
- [ ] TTL/cache invalidation strategy
- [ ] Used by risk or execution validation before submit

## Refs
`data_plane/ingest/coinbase_rest.py`, execution layer
```

---

## Issue 5

### Title
`Storage: QuestDB production path (batch writes, retention, decision traces)`

### Body
```markdown
## Goal
Batched writes, retention policy, failure handling; persist `decision_trace` rows.

## Acceptance
- [ ] Bars + decision_traces written from live path
- [ ] Document backup/restore expectations in `docs/`

## Refs
`data_plane/storage/questdb.py`, `data_plane/storage/schemas.py`
```

---

## Issue 6

### Title
`Storage: Redis TTL and bounded pub/sub`

### Body
```markdown
## Goal
TTL on `nm:bar:*` and state keys; reconnect policy; no unbounded growth.

## Acceptance
- [ ] Documented TTL values in config
- [ ] Integration test with Redis container

## Refs
`data_plane/storage/redis_state.py`
```

---

## Issue 7

### Title
`Storage: Qdrant news_context_memory — version payload + query tests`

### Body
```markdown
## Goal
Embedding model version in payload; verify top-K + symbol + recency; backup notes.

## Acceptance
- [ ] Integration test with Qdrant container
- [ ] Collection schema documented

## Refs
`data_plane/memory/qdrant_memory.py`
```

---

## Issue 8

### Title
`Features: Full pipeline in live path + schema_version parity with backtest`

### Body
```markdown
## Goal
Live loop uses `FeaturePipeline` + same code path as `backtesting/replay.py`.

## Acceptance
- [ ] `live_service` builds Polars frame / feature row from bars
- [ ] Shared function or import test prevents drift

## Refs
`data_plane/features/pipeline.py`, `app/runtime/live_service.py`, `backtesting/replay.py`
```

---

## Issue 9

### Title
`Features: Microstructure + sentiment (FinBERT, frequency, shocks)`

### Body
```markdown
## Goal
Implement spec §5 microstructure and sentiment; wire FinBERT when providers exist.

## Acceptance
- [ ] Feature columns documented and tested
- [ ] No raw text in `OrderIntent` (already validated)

## Refs
`data_plane/features/pipeline.py`, `data_plane/ingest/news_ingest.py`
```

---

## Issue 10

### Title
`Memory: 60s Qdrant retrieval loop → feature vector`

### Body
```markdown
## Goal
`NM_MEMORY_RETRIEVAL_INTERVAL_SECONDS` loop feeding memory aggregates into decisions.

## Acceptance
- [ ] asyncio task in live runner
- [ ] Aggregates match spec (similarity, sentiment, recency)

## Refs
`data_plane/memory/`, `app/config/settings.py`
```

---

## Issue 11

### Title
`Models: Train + persist HMM regime with validated semantic mapping`

### Body
```markdown
## Goal
Fit Gaussian HMM on historical features; persist scaler+model; validate bull/bear/volatile/sideways mapping.

## Acceptance
- [ ] Load artifact in prod inference only
- [ ] Evaluation notebook or script in `models/regime/`

## Refs
`models/regime/hmm_regime.py`
```

---

## Issue 12

### Title
`Models: Replace TFT Ridge surrogate with Temporal Fusion Transformer`

### Body
```markdown
## Goal
Spec calls for TFT; implement PyTorch TFT or document formal deviation in `docs/`.

## Acceptance
- [ ] Multi-horizon outputs + volatility + uncertainty
- [ ] MLflow run logged

## Refs
`models/forecast/tft_forecast.py`
```

---

## Issue 13

### Title
`Models: Route selector thresholds in config + route outcome tests`

### Body
```markdown
## Goal
Move magic numbers to YAML; unit tests for route ranking edge cases.

## Acceptance
- [ ] `DeterministicRouteSelector` reads from `AppSettings` or config file
- [ ] Tests for NO_TRADE vs SCALPING etc.

## Refs
`models/routing/route_selector.py`
```

---

## Issue 14

### Title
`MLflow: Manual promotion only — document and enforce in code`

### Body
```markdown
## Goal
No auto-promotion; documented human gate; registry stub replaced with real workflow.

## Acceptance
- [ ] `models/registry/mlflow_registry.py` behavior matches policy
- [ ] `docs/` describes promotion steps

## Refs
`models/registry/mlflow_registry.py`, `orchestration/`
```

---

## Issue 15

### Title
`Decision: Contract tests for RouteDecision + ActionProposal vs spec`

### Body
```markdown
## Goal
Tests assert `route_id`, `confidence`, `ranking`; action fields vs risk caps per route.

## Acceptance
- [ ] pytest covers each `RouteId`
- [ ] Documented mapping route → order type / expiry

## Refs
`decision_engine/`, `app/contracts/decisions.py`
```

---

## Issue 16

### Title
`Risk: Document limit precedence + implement FLATTEN / REDUCE_ONLY with positions`

### Body
```markdown
## Goal
Replace reduce-only stub with position-aware closes; document order when multiple limits fire.

## Acceptance
- [ ] FLATTEN_ALL issues closing orders per symbol
- [ ] Tests for each `SystemMode`

## Refs
`risk_engine/engine.py`, execution adapters
```

---

## Issue 17

### Title
`Execution: Enforce paper/live adapter from config in one place`

### Body
```markdown
## Goal
`execution_paper_adapter` / `execution_live_adapter` respected; no alternate construction paths.

## Acceptance
- [ ] Single factory used by `ExecutionService` and tests
- [ ] Misconfig fails fast at startup

## Refs
`execution/router.py`, `execution/service.py`
```

---

## Issue 18

### Title
`Execution: Alpaca paper — errors, symbol map, reconciliation`

### Body
```markdown
## Goal
Production-grade Alpaca adapter: retry policy, clear errors, BTC-USD→BTCUSD mapping tests, periodic position reconcile.

## Acceptance
- [ ] Integration test with paper API optional in CI (secret)
- [ ] Logs redact secrets

## Refs
`execution/adapters/alpaca_paper.py`
```

---

## Issue 19

### Title
`Execution: Coinbase live — signed orders, cancel, fills, idempotency`

### Body
```markdown
## Goal
Remove `pending_implementation` path; implement CDP/JWT signing per current Coinbase Advanced Trade API.

## Acceptance
- [ ] Submit, cancel, fetch order status
- [ ] Client order id / idempotency key
- [ ] No live orders without valid risk HMAC on intent

## Refs
`execution/adapters/coinbase_live.py`
```

---

## Issue 20

### Title
`Runtime: Full live pipeline + QuestDB audit persistence`

### Body
```markdown
## Goal
Replace skeleton `live_service` with WS→bars→features→models→risk→ExecutionService→QuestDB traces.

## Acceptance
- [ ] One runnable entrypoint documented in README
- [ ] Decision traces persisted, not only logged

## Refs
`app/runtime/live_service.py`, `data_plane/storage/`
```

---

## Issue 21

### Title
`Runtime: Graceful shutdown + documented flatten behavior`

### Body
```markdown
## Goal
SIGTERM handling; cancel tasks; optional flatten-on-shutdown policy.

## Acceptance
- [ ] Documented in `docs/`
- [ ] asyncio shutdown tested

## Refs
`app/runtime/live_service.py`
```

---

## Issue 22

### Title
`Backtest: CI test — live and replay share decision/risk entrypoints`

### Body
```markdown
## Goal
Prevent drift between `replay_decisions` and live loop imports.

## Acceptance
- [ ] Single module or import guard test in `tests/`

## Refs
`backtesting/replay.py`, `decision_engine/pipeline.py`
```

---

## Issue 23

### Title
`Backtest: Simulator — fees, slippage, reproducible RNG`

### Body
```markdown
## Goal
Expand `simulator.py` / `PortfolioTracker` for realistic backtests per config.

## Acceptance
- [ ] Seed-controlled runs
- [ ] Config from `backtesting_slippage_bps`

## Refs
`backtesting/simulator.py`, `backtesting/portfolio.py`
```

---

## Issue 24

### Title
`Control plane: Streamlit pages (Live, Regimes, Routes, Models, Logs, Emergency)`

### Body
```markdown
## Goal
Spec §14 dashboard pages wired to FastAPI + state.

## Acceptance
- [ ] Multi-page Streamlit app
- [ ] Emergency actions call authenticated API

## Refs
`control_plane/dashboard.py`, `control_plane/api.py`
```

---

## Issue 25

### Title
`Observability: Stage latency metrics + feed health + order success`

### Body
```markdown
## Goal
Wire Prometheus metrics for ingest, feature, model, risk, submit stages; PnL/drawdown gauges.

## Acceptance
- [ ] Grafana scrape target documented
- [ ] Correlation id in structured logs

## Refs
`observability/metrics.py`, `observability/logging.py`
```

---

## Issue 26

### Title
`Observability: Ship JSON logs to Loki + Grafana dashboards/alerts`

### Body
```markdown
## Goal
Loki driver or Promtail config; dashboards for stale data, disconnects, risk blocks, order failures.

## Acceptance
- [ ] `infra/` or `docs/` with example configs

## Refs
`infra/docker-compose.yml`
```

---

## Issue 27

### Title
`Orchestration: Prefect nightly retrain — data → train → MLflow → manual gate`

### Body
```markdown
## Goal
Replace stub with deployable flow; no auto model promotion.

## Acceptance
- [ ] Walk-forward / no-leakage validation in flow
- [ ] Document manual promotion after evaluate

## Refs
`orchestration/nightly_retrain.py`
```

---

## Issue 28

### Title
`Ops: Secrets rotation + runbooks (incident, flatten, restore)`

### Body
```markdown
## Goal
Short runbooks in `docs/` for keys, flatten procedure, QuestDB restore.

## Acceptance
- [ ] Paper vs live credential separation documented

## Refs
`docs/PRODUCTION_HARDENING.md`
```

---

## Issue 29

### Title
`CI: Integration tests — Redis, QuestDB, Qdrant containers`

### Body
```markdown
## Goal
pytest integration job with services from compose or testcontainers.

## Acceptance
- [ ] GitHub Actions workflow (or documented local command)

## Refs
`tests/`
```

---

## Issue 30

### Title
`CI: E2E paper trade dry run (optional secrets) + release checklist`

### Body
```markdown
## Goal
Optional scheduled E2E with Alpaca paper; release process tags model version in deploy config.

## Acceptance
- [ ] Document required secrets
- [ ] `CHANGELOG` or release doc template

## Refs
`.github/` (if workflow added)
```

---

## Issue 31

### Title
`Infra: Compose stack completeness (MLflow, Prefect, Streamlit as needed)`

### Body
```markdown
## Goal
Align `infra/docker-compose.yml` with spec services you run in dev/prod (MLflow, Prefect, etc.).

## Acceptance
- [ ] README “stack up” matches reality
- [ ] Ports documented

## Refs
`infra/docker-compose.yml`
```

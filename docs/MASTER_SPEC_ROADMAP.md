# Master Spec V3 — roadmap

This is the **single place** for Master Spec V3 work: production checklist, narrative gaps, numbered issues, and optional GitHub sync. Deep links for risk order, QuestDB, shutdown, and candles remain in separate short docs (see bottom).

---

## Contents

- [Production hardening checklist](#production-hardening-checklist)
- [Narrative (commentary)](#narrative-commentary)
- [Issue log](#issue-log)
- [Optional: GitHub issues](#optional-github-issues)

---

## Production hardening checklist

Use this as the **single checklist** to reach full spec compliance. Check items off in PRs as you complete them.

**Issue log:** [Issue log](#issue-log) — **Not started** | **Pending** | **Completed**.  
**Narrative:** [Narrative (commentary)](#narrative-commentary)  
**Risk order:** [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md)  
**QuestDB traces:** [`QUESTDB_TRACES.md`](QUESTDB_TRACES.md)  
**Shutdown:** [`GRACEFUL_SHUTDOWN.md`](GRACEFUL_SHUTDOWN.md)  
**Candles:** [`COINBASE_GRANULARITY.md`](COINBASE_GRANULARITY.md)

**Epic goal:** Coinbase-only data, Alpaca paper / Coinbase live execution, identical decision + risk path for paper and live, risk final authority, auditable actions, no auto model promotion, backtest ≈ live.

---

## 1. Spec compliance gates (non-negotiables)

- [x] **Coinbase-only data:** `scripts/ci_spec_compliance.sh`
- [x] **Risk HMAC:** `NM_RISK_SIGNING_SECRET` + `execution.intent_gate`
- [x] **No raw text → trades:** `OrderIntent` metadata validator
- [ ] **No auto model promotion:** document in `docs/MLFLOW_PROMOTION.md` (stub) + enforce in automation
- [x] **Audit trail:** `decision_trace` + log; optional QuestDB when `NM_QUESTDB_PERSIST_DECISION_TRACES=true`

---

## 2. Coinbase market data

- [x] **WebSocket health:** `last_message_at` / `message_count`; feed age blocks in `RiskEngine` first
- [x] **REST (partial):** retries + backoff on 429/5xx; see [`COINBASE_GRANULARITY.md`](COINBASE_GRANULARITY.md)
- [x] **Normalizers (partial):** fixture test + `NORMALIZER_UNKNOWN` metric; `best_bid`/`best_ask` on ticker
- [x] **Product metadata (partial):** `ProductMetadataCache` + `product_tradable` in risk

---

## 3. Storage (QuestDB, Redis, Qdrant)

- [x] **QuestDB (partial):** `insert_decision_trace_dict`; full batch/backup TBD
- [x] **Redis:** bar key TTL (`redis.bar_ttl_seconds`)
- [ ] **Qdrant:** version payload + integration tests

---

## 4. Feature pipeline & memory

- [x] **Parity:** `run_decision_tick` + `enrich_bars_last_row` (live rolling minute bars + replay cumulative window)
- [x] **Live features:** Polars pipeline on rolling bars + tick overlay (`feature_schema_version`)
- [x] **Memory (partial):** 60s asyncio task in live (placeholder mem dict); real Qdrant TBD

---

## 5. Models (regime, forecast, routing)

- [ ] **HMM:** train + persist
- [ ] **TFT:** PyTorch or documented deviation
- [x] **Route selector:** thresholds in `routing` YAML
- [ ] **MLflow:** real runs + manual promotion doc

---

## 6. Decision & action

- [x] **`RouteDecision`:** Pydantic contract (route_id, confidence, ranking)
- [ ] **Action generator:** full per-route matrix vs risk tests

---

## 7. Risk engine

- [x] **Precedence:** [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md)
- [x] **Feed stale:** `feed_last_message_at` + `nm_feed_stale_blocks_total`
- [x] **System modes (partial):** FLATTEN_ALL + REDUCE_ONLY position-aware; PAUSE/MAINTENANCE tests TBD
- [x] **Positions (paper, partial):** optional Alpaca `fetch_positions` on startup + interval when `position_reconcile_enabled`

---

## 8. Execution

- [x] **Router:** alpaca/coinbase name validation
- [x] **Alpaca (partial):** submit/cancel/fetch retries + safe logs + symbol map tests; optional CI vs paper API TBD
- [ ] **Coinbase live:** signed orders

---

## 9. Live runtime service

- [x] **Pipeline:** WS → features → `run_decision_tick` → trace → optional QuestDB → execution
- [x] **Shutdown (partial):** SIGINT/SIGTERM — [`GRACEFUL_SHUTDOWN.md`](GRACEFUL_SHUTDOWN.md)

---

## 10. Backtesting

- [x] **Shared step:** `run_decision_tick` in `replay_decisions`
- [x] **Simulator (partial):** fees + slippage + optional noise from YAML; `track_portfolio` in replay; seeded RNG for noise
- [ ] **Multi-symbol portfolio replay:** Issue 32
- [ ] **Risk vs solvency in replay:** Issue 33
- [ ] **Simulator semantics doc:** Issue 34

---

## 11. Control plane

- [x] **FastAPI:** mutating auth when API key set
- [x] **Streamlit (shell):** `control_plane/Home.py` + `pages/*`

---

## 12. Observability

- [x] **Metrics (partial):** `FEED_STALE_BLOCKS`, `NORMALIZER_UNKNOWN`, order counters
- [ ] **Loki + Grafana:** deploy wiring

---

## 13–15. Retraining, security, CI

- [ ] Prefect flow, runbooks, integration CI — unchanged

---

## Definition of done (spec-complete)

All sections above checked; release PR links this file revision.

---

## Narrative (commentary)

This is a **running narrative** for humans and agents: what the spec asked for, what the code does today, and what still separates “scaffold” from “production.”

## What the spec is optimizing for

You asked for a **single Coinbase truth** for prices, **Alpaca only for paper fills**, the same **decision + risk** path for paper and live, **models as signals** (not magic strings that place orders), and a **risk engine that cannot be skipped**. The repo now encodes several of those as **mechanisms** (HMAC on `OrderIntent`, metadata ban on raw news text, CI grep for Alpaca data imports) rather than only as documentation.

## What landed in the latest push

- **Routing thresholds** live in `app/config/default.yaml` under `routing` (and `NM_ROUTING_*`), consumed by `DeterministicRouteSelector`. That closes the “magic numbers only in Python” gap for route selection.
- **Execution router** validates `execution_paper_adapter` / `execution_live_adapter` against **alpaca** / **coinbase** so misconfiguration fails fast.
- **Live loop** uses **message time** from tickers/trades for `data_timestamp` and infers **spread_bps** from ticker bid/ask or L2 when possible — so stale-data and spread limits in `RiskEngine` are tied to real feed shape, not `datetime.now()` only.
- **Redis** latest bar keys get a **TTL** (`redis.bar_ttl_seconds`) so keys do not grow forever.
- **Memory loop helper** `run_memory_retrieval_loop` implements the **60s cadence** pattern against Qdrant; you still need to start it alongside the live runner and pass a real query embedding when FinBERT/news encoders exist.
- **Sentiment feature hook** `FeaturePipeline.sentiment_features()` holds the three spec slots (FinBERT, frequency, shock) until NLP is wired.
- **Streamlit** multipage shell: `control_plane/Home.py` + `pages/` for Live, Regimes, Routes, Models, Logs, Emergency — each page is thin until you bind QuestDB/Loki.

## Rolling bars + risk modes (current batch)

- **`RollingMinuteBars`**: 1m OHLCV from ticks per symbol; **`enrich_bars_last_row`** matches replay.
- **Live**: merges Polars feature row with tick **overlay** (microstructure, memory placeholders).
- **Replay**: cumulative raw OHLCV slice → same `enrich_bars_last_row` + `run_decision_tick`; tracks position from simulated trades.
- **FLATTEN_ALL** / **REDUCE_ONLY**: position-aware via `position_signed_qty` (`Decimal`).

## Latest batch (queue continuation)

- **`run_decision_tick`** is the single decision+risk step for **live** and **replay** (`decision_engine/run_step.py`).
- **`RiskEngine`** checks **`feed_last_message_at`** first (before bar timestamp); **`nm_feed_stale_blocks_total`** counter.
- **Coinbase REST** retries with exponential backoff on 429/5xx.
- **`ProductMetadataCache`** + **`product_tradable`** gate in risk; **`live_service`** wires both.
- **QuestDB:** enable **`NM_QUESTDB_PERSIST_DECISION_TRACES`** to persist full JSON traces (`docs/QUESTDB_TRACES.md`).
- **Live loop:** `FeaturePipeline` + **`feature_row_from_tick`**, 60s memory **placeholder** task, SIGINT/SIGTERM stop (`docs/GRACEFUL_SHUTDOWN.md`).
- **Tests:** feed-stale risk, normalizer fixture, backtest/live parity imports.

## Honest gaps

- **Coinbase live** signed orders; **TFT** PyTorch; **Qdrant** real embeddings in the 60s loop; **Prefect**; **Grafana/Loki** wiring.
- **Risk modes:** PAUSE / MAINTENANCE matrix tests still outstanding (Issue 16).
- **Alpaca paper:** retries + symbol helpers + optional venue reconcile are in code; optional CI integration against the paper API still not added (Issue 18).

## Latest batch (Alpaca paper + live reconcile)

- **`execution/alpaca_util.py`:** `to_alpaca_crypto_symbol` / `from_alpaca_crypto_symbol`, `redact_secrets_for_log` for safe retry logs.
- **`AlpacaPaperExecutionAdapter`:** bounded retries with exponential backoff + jitter on transient failures; `fetch_positions` maps Alpaca symbols back to Coinbase-style `BTC-USD` keys.
- **`live_service`:** optional **position reconcile** in paper mode (`NM_POSITION_RECONCILE_ENABLED` / `execution.position_reconcile_*`): startup fetch + periodic refresh from Alpaca so `position_signed_qty` matches the broker when enabled. In-memory updates after fills still apply when reconcile is off.
- **README:** documents `python -m app.runtime.live_service` and reconcile env vars.

## Latest batch (backtest simulator — Issue 23)

- **`backtesting`:** `fee_bps`, `slippage_noise_bps`, `rng_seed`, `initial_cash_usd` in config (`NM_BACKTESTING_*`).
- **`replay_decisions(..., track_portfolio=True)`** applies simulated fill prices (slippage ± optional noise with seeded `Random`), fees on notional, and updates `PortfolioTracker`; rows include `portfolio_cash`, `equity_mark`, etc.
- **New gaps logged:** Issues **32** (multi-symbol replay), **33** (risk vs cash solvency), **34** (fee/slippage doc).

## How to use the issue log

The [Issue log](#issue-log) below uses **Not started**, **Pending**, **Completed**. Move items as you merge work. The epic stays **Pending** until everything that matters for your definition of V1 is **Completed**.

When in doubt, prefer **Pending** over **Completed** — “Completed” should mean you would defend the implementation in a production review.

---

## Issue log

Local tracking file in **GitHub issue style** (title + body per item).

## Status values (pick one per item)

| Status | Meaning |
|--------|---------|
| **Not started** | No meaningful implementation yet |
| **Pending** | In progress, partial, or needs wiring / hardening |
| **Completed** | Done to spec for this codebase (may still need ops/CI) |

Also see: [Production hardening checklist](#production-hardening-checklist)

---

### Epic: Master Spec V3 — remaining work

**Status:** Pending  
**Type:** Epic

Parent tracker for NautilusMonster V3 spec compliance. Close when all work items below are done and the [Production hardening checklist](#production-hardening-checklist) above is fully checked.

---

### Issue 1 — Data: Wire Coinbase WS feed health to risk (stale data)

**Status:** Completed

## Goal

Align live feed staleness with `NM_RISK_STALE_DATA_SECONDS` using `CoinbaseWebSocketClient.last_message_at` / gaps, not only synthetic timestamps in `live_service`.

## Acceptance

- [x] Risk blocks when `feed_last_message_at` age exceeds stale threshold (`risk_engine.engine`)
- [x] Prometheus `nm_feed_stale_blocks_total` on feed-stale block

## Refs

`data_plane/ingest/coinbase_ws.py`, `risk_engine/engine.py`, `app/runtime/live_service.py`

---

### Issue 2 — Data: Harden Coinbase REST (rate limits, errors, candles for V1 symbols)

**Status:** Pending

## Goal

Production-ready REST client for candles/metadata: retries, rate limits, pagination, clear errors.

## Acceptance

- [ ] BTC-USD, ETH-USD, SOL-USD candle fetch validated against live or recorded fixtures
- [ ] Documented granularity mapping for bar pipeline

## Refs

`data_plane/ingest/coinbase_rest.py`

---

### Issue 3 — Data: Normalizer tests from recorded Coinbase WS payloads

**Status:** Pending

## Goal

Contract tests using real JSON fixtures; unknown messages increment metrics, never corrupt state.

## Acceptance

- [x] Fixture under `tests/fixtures/coinbase_ws/` (ticker sample)
- [x] `test_normalizer_fixtures.py`; unknown messages increment `NORMALIZER_UNKNOWN`

## Refs

`data_plane/ingest/normalizers.py`

---

### Issue 4 — Data: Product metadata cache (tick size, min size, status)

**Status:** Completed

## Goal

Cache Coinbase product metadata for sizing and filters.

## Acceptance

- [x] TTL in `ProductMetadataCache` (default 300s)
- [x] `product_tradable` passed into `RiskEngine` from `live_service`

## Refs

`data_plane/ingest/coinbase_rest.py`, execution layer

---

### Issue 5 — Storage: QuestDB production path (batch writes, retention, decision traces)

**Status:** Pending

## Goal

Batched writes, retention policy, failure handling; persist `decision_trace` rows.

## Acceptance

- [x] `insert_decision_trace_dict` + `NM_QUESTDB_PERSIST_DECISION_TRACES` from live path
- [ ] Batched bars + backup runbook (`docs/QUESTDB_TRACES.md` partial)

## Refs

`data_plane/storage/questdb.py`, `data_plane/storage/schemas.py`

---

### Issue 6 — Storage: Redis TTL and bounded pub/sub

**Status:** Completed

## Goal

TTL on `nm:bar:*` and state keys; reconnect policy; no unbounded growth.

## Acceptance

- [ ] Documented TTL values in config
- [ ] Integration test with Redis container

## Refs

`data_plane/storage/redis_state.py`

---

### Issue 7 — Storage: Qdrant news_context_memory — version payload + query tests

**Status:** Pending

## Goal

Embedding model version in payload; verify top-K + symbol + recency; backup notes.

## Acceptance

- [ ] Integration test with Qdrant container
- [ ] Collection schema documented

## Refs

`data_plane/memory/qdrant_memory.py`

---

### Issue 8 — Features: Full pipeline in live path + schema_version parity with backtest

**Status:** Completed

## Goal

Live loop uses `FeaturePipeline` + same code path as `backtesting/replay.py`.

## Acceptance

- [x] `RollingMinuteBars` + `enrich_bars_last_row` in live; replay uses same helper on cumulative OHLCV
- [x] `feature_schema_version` on enriched rows; tick overlay merged via `merge_feature_overlays`
- [x] Parity tests import `run_decision_tick` + `enrich_bars_last_row` in both paths

## Refs

`data_plane/features/pipeline.py`, `app/runtime/live_service.py`, `backtesting/replay.py`, `decision_engine/feature_frame.py`

---

### Issue 9 — Features: Microstructure + sentiment (FinBERT, frequency, shocks)

**Status:** Pending

## Goal

Implement spec §5 microstructure and sentiment; wire FinBERT when providers exist.

## Acceptance

- [ ] Feature columns documented and tested
- [ ] No raw text in `OrderIntent` (already validated)

## Refs

`data_plane/features/pipeline.py`, `data_plane/ingest/news_ingest.py`

---

### Issue 10 — Memory: 60s Qdrant retrieval loop → feature vector

**Status:** Pending

## Goal

`NM_MEMORY_RETRIEVAL_INTERVAL_SECONDS` loop feeding memory aggregates into decisions.

## Acceptance

- [x] asyncio task in `live_service` (`_memory_tick_loop` — placeholder zeros until Qdrant)
- [ ] Real Qdrant embeddings + `run_memory_retrieval_loop` wired

## Refs

`data_plane/memory/`, `app/config/settings.py`

---

### Issue 11 — Models: Train + persist HMM regime with validated semantic mapping

**Status:** Not started

## Goal

Fit Gaussian HMM on historical features; persist scaler+model; validate bull/bear/volatile/sideways mapping.

## Acceptance

- [ ] Load artifact in prod inference only
- [ ] Evaluation notebook or script in `models/regime/`

## Refs

`models/regime/hmm_regime.py`

---

### Issue 12 — Models: Replace TFT Ridge surrogate with Temporal Fusion Transformer

**Status:** Not started

## Goal

Spec calls for TFT; implement PyTorch TFT or document formal deviation in `docs/`.

## Acceptance

- [ ] Multi-horizon outputs + volatility + uncertainty
- [ ] MLflow run logged

## Refs

`models/forecast/tft_forecast.py`

---

### Issue 13 — Models: Route selector thresholds in config + route outcome tests

**Status:** Completed

## Goal

Move magic numbers to YAML; unit tests for route ranking edge cases.

## Acceptance

- [ ] `DeterministicRouteSelector` reads from `AppSettings` or config file
- [ ] Tests for NO_TRADE vs SCALPING etc.

## Refs

`models/routing/route_selector.py`

---

### Issue 14 — MLflow: Manual promotion only — document and enforce in code

**Status:** Not started

## Goal

No auto-promotion; documented human gate; registry stub replaced with real workflow.

## Acceptance

- [ ] `models/registry/mlflow_registry.py` behavior matches policy
- [ ] `docs/` describes promotion steps

## Refs

`models/registry/mlflow_registry.py`, `orchestration/`

---

### Issue 15 — Decision: Contract tests for RouteDecision + ActionProposal vs spec

**Status:** Pending

## Goal

Tests assert `route_id`, `confidence`, `ranking`; action fields vs risk caps per route.

## Acceptance

- [ ] pytest covers each `RouteId`
- [ ] Documented mapping route → order type / expiry

## Refs

`decision_engine/`, `app/contracts/decisions.py`

---

### Issue 16 — Risk: Document limit precedence + implement FLATTEN / REDUCE_ONLY with positions

**Status:** Pending

## Goal

Replace reduce-only stub with position-aware closes; document order when multiple limits fire.

## Acceptance

- [x] FLATTEN_ALL: full close market action when `position_signed_qty` ≠ 0
- [x] REDUCE_ONLY: block adds; allow reduce side; cap qty to position
- [x] `tests/test_risk_modes_position.py`; live loop tracks per-symbol position after fills
- [x] Fetch positions from Alpaca on startup + periodic reconcile when `execution.position_reconcile_enabled` (paper)
- [ ] PAUSE / MAINTENANCE matrix tests

## Refs

`risk_engine/engine.py`, execution adapters, `app/runtime/live_service.py`

---

### Issue 17 — Execution: Enforce paper/live adapter from config in one place

**Status:** Completed

## Goal

`execution_paper_adapter` / `execution_live_adapter` respected; no alternate construction paths.

## Acceptance

- [ ] Single factory used by `ExecutionService` and tests
- [ ] Misconfig fails fast at startup

## Refs

`execution/router.py`, `execution/service.py`

---

### Issue 18 — Execution: Alpaca paper — errors, symbol map, reconciliation

**Status:** Pending

## Goal

Production-grade Alpaca adapter: retry policy, clear errors, BTC-USD→BTCUSD mapping tests, periodic position reconcile.

## Acceptance

- [x] Retry with backoff on transient Alpaca/network errors (`submit_order`, `cancel_order`, `fetch_positions`)
- [x] Symbol map helpers + unit tests (`execution/alpaca_util.py`, `tests/test_alpaca_util.py`)
- [x] Log lines use redacted exception text (`safe_exc_message`)
- [x] Optional `position_reconcile_enabled` + interval: startup + periodic `fetch_positions` → `positions` in `live_service` (paper mode)
- [ ] Integration test with paper API optional in CI (secret)

## Refs

`execution/adapters/alpaca_paper.py`, `execution/alpaca_util.py`, `app/runtime/live_service.py`

---

### Issue 19 — Execution: Coinbase live — signed orders, cancel, fills, idempotency

**Status:** Not started

## Goal

Remove `pending_implementation` path; implement CDP/JWT signing per current Coinbase Advanced Trade API.

## Acceptance

- [ ] Submit, cancel, fetch order status
- [ ] Client order id / idempotency key
- [ ] No live orders without valid risk HMAC on intent

## Refs

`execution/adapters/coinbase_live.py`

---

### Issue 20 — Runtime: Full live pipeline + QuestDB audit persistence

**Status:** Pending

## Goal

Replace skeleton `live_service` with WS→bars→features→models→risk→ExecutionService→QuestDB traces.

## Acceptance

- [x] Runnable entrypoint: `python -m app.runtime.live_service` (documented in README)
- [x] Decision traces persisted when `NM_QUESTDB_PERSIST_DECISION_TRACES=true` / YAML questdb flag

## Refs

`app/runtime/live_service.py`, `data_plane/storage/`

---

### Issue 21 — Runtime: Graceful shutdown + documented flatten behavior

**Status:** Pending

## Goal

SIGTERM handling; cancel tasks; optional flatten-on-shutdown policy.

## Acceptance

- [x] `docs/GRACEFUL_SHUTDOWN.md`
- [ ] Signal handling tested in CI (optional)

## Refs

`app/runtime/live_service.py`

---

### Issue 22 — Backtest: CI test — live and replay share decision/risk entrypoints

**Status:** Completed

## Goal

Prevent drift between `replay_decisions` and live loop imports.

## Acceptance

- [ ] Single module or import guard test in `tests/`

## Refs

`backtesting/replay.py`, `decision_engine/pipeline.py`

---

### Issue 23 — Backtest: Simulator — fees, slippage, reproducible RNG

**Status:** Completed

## Goal

Expand `simulator.py` / `PortfolioTracker` for realistic backtests per config.

## Acceptance

- [x] Seed-controlled slippage noise via `backtesting.rng_seed` / `NM_BACKTESTING_RNG_SEED` (`make_replay_rng`)
- [x] Slippage + fee from YAML: `slippage_bps`, `fee_bps`, `slippage_noise_bps`, `initial_cash_usd`
- [x] `replay_decisions(..., track_portfolio=True)` applies fills + fees + optional equity columns

## Refs

`backtesting/simulator.py`, `backtesting/portfolio.py`, `backtesting/replay.py`, `backtesting/execution_params.py`

---

### Issue 24 — Control plane: Streamlit pages (Live, Regimes, Routes, Models, Logs, Emergency)

**Status:** Pending

## Goal

Spec §14 dashboard pages wired to FastAPI + state.

## Acceptance

- [ ] Multi-page Streamlit app
- [ ] Emergency actions call authenticated API

## Refs

`control_plane/Home.py`, `control_plane/pages/`, `control_plane/api.py`

---

### Issue 25 — Observability: Stage latency metrics + feed health + order success

**Status:** Pending

## Goal

Wire Prometheus metrics for ingest, feature, model, risk, submit stages; PnL/drawdown gauges.

## Acceptance

- [ ] Grafana scrape target documented
- [ ] Correlation id in structured logs

## Refs

`observability/metrics.py`, `observability/logging.py`

---

### Issue 26 — Observability: Ship JSON logs to Loki + Grafana dashboards/alerts

**Status:** Not started

## Goal

Loki driver or Promtail config; dashboards for stale data, disconnects, risk blocks, order failures.

## Acceptance

- [ ] `infra/` or `docs/` with example configs

## Refs

`infra/docker-compose.yml`

---

### Issue 27 — Orchestration: Prefect nightly retrain — data → train → MLflow → manual gate

**Status:** Not started

## Goal

Replace stub with deployable flow; no auto model promotion.

## Acceptance

- [ ] Walk-forward / no-leakage validation in flow
- [ ] Document manual promotion after evaluate

## Refs

`orchestration/nightly_retrain.py`

---

### Issue 28 — Ops: Secrets rotation + runbooks (incident, flatten, restore)

**Status:** Not started

## Goal

Short runbooks in `docs/` for keys, flatten procedure, QuestDB restore.

## Acceptance

- [ ] Paper vs live credential separation documented

## Refs

[`MASTER_SPEC_ROADMAP.md` — Production hardening checklist](#production-hardening-checklist)

---

### Issue 29 — CI: Integration tests — Redis, QuestDB, Qdrant containers

**Status:** Not started

## Goal

pytest integration job with services from compose or testcontainers.

## Acceptance

- [ ] GitHub Actions workflow (or documented local command)

## Refs

`tests/`

---

### Issue 30 — CI: E2E paper trade dry run (optional secrets) + release checklist

**Status:** Not started

## Goal

Optional scheduled E2E with Alpaca paper; release process tags model version in deploy config.

## Acceptance

- [ ] Document required secrets
- [ ] `CHANGELOG` or release doc template

## Refs

`.github/` (if workflow added)

---

### Issue 31 — Infra: Compose stack completeness (MLflow, Prefect, Streamlit as needed)

**Status:** Not started

## Goal

Align `infra/docker-compose.yml` with spec services you run in dev/prod (MLflow, Prefect, etc.).

## Acceptance

- [ ] README “stack up” matches reality
- [ ] Ports documented

## Refs

`infra/docker-compose.yml`

---

### Issue 32 — Backtest: Multi-symbol replay with one shared portfolio

**Status:** Not started

## Goal

`replay_decisions` is single-symbol today; for portfolio-level backtests, run multiple symbols against one `PortfolioTracker` without double-counting cash or splitting logic ad hoc.

## Acceptance

- [ ] API design (e.g. `replay_multi` or explicit `portfolio` + per-symbol bar frames)
- [ ] Tests with two symbols and non-overlapping trade times

## Refs

`backtesting/replay.py`, `backtesting/portfolio.py`

---

### Issue 33 — Backtest: RiskEngine vs simulated solvency

**Status:** Not started

## Goal

Replay uses simulated cash for fees/slippage, but `RiskEngine` does not see wallet balance — trades can be simulated even if insolvent. Align or document (e.g. optional cash check in replay, or separate “paper equity” risk mode).

## Acceptance

- [ ] Document current behavior in `docs/` or README backtest section
- [ ] Optional: pass `available_cash` into risk evaluation for replay-only runs

## Refs

`backtesting/replay.py`, `risk_engine/engine.py`

---

### Issue 34 — Docs: Backtest simulator semantics (fee model)

**Status:** Not started

## Goal

Short reference for how `fee_bps`, `slippage_bps`, and `slippage_noise_bps` combine (per-fill fee on notional, half-spread slippage).

## Acceptance

- [ ] `docs/BACKTESTING_SIMULATOR.md` or section in README

## Refs

`backtesting/simulator.py`, `app/config/default.yaml` (`backtesting:`)

---

### Issue 35 — Data: Coinbase Advanced Trade REST auth for candles/products

**Status:** Not started

## Goal

`CoinbaseRESTClient` public candle/product calls return **401** without JWT in some environments. Either sign requests with CDP keys (read-only) or document that market data uses Exchange public API until signed.

## Acceptance

- [ ] Document behavior in `COINBASE_GRANULARITY.md` or REST module docstring
- [ ] Optional: JWT helper reusing `NM_COINBASE_*` for `/products` and candles

## Refs

`data_plane/ingest/coinbase_rest.py`, `scripts/smoke_credentials.py`

---

## Optional: GitHub issues

**Canonical backlog:** this file — see [Issue log](#issue-log).

Statuses: **Not started** | **Pending** | **Completed** (see table under [Issue log](#issue-log)).

To create real GitHub issues, copy each `### Issue N — …` section into **New issue**, or run `scripts/create_github_issues.sh` if you use `gh` CLI.

**Narrative (commentary vs spec):** [Narrative (commentary)](#narrative-commentary)

---

## Related reference docs

These are **not** duplicate backlogs; they document behavior and precedence.

- [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md)
- [`QUESTDB_TRACES.md`](QUESTDB_TRACES.md)
- [`GRACEFUL_SHUTDOWN.md`](GRACEFUL_SHUTDOWN.md)
- [`COINBASE_GRANULARITY.md`](COINBASE_GRANULARITY.md)
- [`MLFLOW_PROMOTION.md`](MLFLOW_PROMOTION.md)

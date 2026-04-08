# Production hardening — Master Spec V3

Use this as the **single checklist** to reach full spec compliance. Check items off in PRs as you complete them.

**Issue log:** [`ISSUE_LOG.md`](ISSUE_LOG.md) — **Not started** | **Pending** | **Completed**.  
**Narrative:** [`COMMENTARY.md`](COMMENTARY.md)  
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
- [ ] **Simulator:** seeds + fees

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

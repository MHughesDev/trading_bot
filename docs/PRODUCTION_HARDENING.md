# Production hardening — Master Spec V3

Use this as the **single checklist** to reach full spec compliance. Check items off in PRs as you complete them.

**Issue log (GitHub-style, 1–31 + epic):** [`ISSUE_LOG.md`](ISSUE_LOG.md) — statuses **Not started** | **Pending** | **Completed**. Narrative: [`COMMENTARY.md`](COMMENTARY.md).

**Epic goal:** Behavior matches Master Spec V3: Coinbase-only market data, Alpaca paper / Coinbase live execution, identical decision path for paper and live, risk engine as final authority, auditable actions, no auto model promotion, backtest ≈ live.

---

## 1. Spec compliance gates (non-negotiables)

- [x] **Coinbase-only data:** `scripts/ci_spec_compliance.sh` rejects `alpaca.data*` imports outside `alpaca_paper.py`.
- [x] **Risk cannot be bypassed:** `NM_RISK_SIGNING_SECRET` set → `OrderIntent` must carry valid HMAC (`risk_engine.signing`); adapters call `execution.intent_gate`. Tests in `tests/test_spec_compliance.py`.
- [x] **No raw text → trades:** `OrderIntent` metadata validator rejects keys like `headline` / `raw_text` (see `app/contracts/orders.py`).
- [ ] **No auto model promotion:** MLflow (or registry) logs/evaluates only; promotion is manual and documented.
- [x] **Audit trail:** `decision_engine.audit.decision_trace()` builds JSON blobs; `app/runtime/live_service.py` logs one trace per tick (wire to QuestDB next).

---

## 2. Coinbase market data

- [x] **WebSocket (partial):** `CoinbaseWebSocketClient` tracks `last_message_at` / `message_count` for health; reconnect loop exists. Full stale-vs-risk wiring still TODO.
- [ ] **REST:** Rate limits, errors, pagination; candle granularity aligned with V1 assets (BTC-USD, ETH-USD, SOL-USD).
- [ ] **Normalizers:** Tests against recorded real payloads; unknown messages dropped with metrics, not silent corruption.
- [ ] **Product metadata:** Cached tick size, min size, product status for sizing and filters.

---

## 3. Storage (QuestDB, Redis, Qdrant)

- [ ] **QuestDB:** Production DDL, retention, backups, batched writes, failure handling.
- [ ] **Redis:** TTL strategy, pub/sub resilience, no unbounded keys.
- [ ] **Qdrant:** `news_context_memory` schema/version; backup; embedding model version in payload; top-K + symbol + recency query verified.

---

## 4. Feature pipeline & memory

- [ ] **Parity:** Same feature code for live and backtest; version feature `schema_version` where applicable.
- [ ] **Spec features:** Returns, vol windows, RSI, MACD, ATR, ADX, EMA spread, VWAP distance; microstructure (spread, imbalance, volume delta, liquidity pressure); sentiment (FinBERT, frequency, shocks) when providers exist.
- [ ] **Memory:** 60s retrieval loop; aggregated similarity, sentiment, recency-weighted signals fed into features.

---

## 5. Models (regime, forecast, routing)

- [ ] **HMM:** Trained 4-state Gaussian HMM; semantic mapping to bull / bear / volatile / sideways validated; persisted scaler + model; inference-only in prod.
- [ ] **TFT:** Replace Ridge surrogate with **Temporal Fusion Transformer** (or document explicit spec deviation and get sign-off).
- [ ] **Route selector:** Deterministic V1 scoring locked in config; evaluation vs NO_TRADE / SCALPING / INTRADAY / SWING.
- [ ] **MLflow:** Experiment tracking; artifacts; manual promotion workflow documented.

---

## 6. Decision & action

- [ ] **`RouteDecision`:** Always includes `route_id`, `confidence`, `ranking` per spec.
- [ ] **Action generator:** Direction, size, stop distance, order type, expiry per route; consistent with risk limits.

---

## 7. Risk engine

- [ ] **Hard limits:** Max exposure, per-symbol, drawdown, spread, stale data — precedence documented when multiple trigger.
- [ ] **System modes:** `RUNNING`, `PAUSE_NEW_ENTRIES`, `REDUCE_ONLY`, `FLATTEN_ALL`, `MAINTENANCE` — defined behavior + tests (especially flatten / reduce-only).

---

## 8. Execution

- [ ] **Router:** `paper` → Alpaca adapter only; `live` → Coinbase adapter; config-driven, single entry.
- [ ] **Alpaca paper:** Production error handling, symbol map (e.g. BTC-USD → venue symbol), reconciliation via `fetch_positions`.
- [ ] **Coinbase live:** **Signed** Advanced Trade orders (CDP/JWT per current API), cancel, fills, idempotency; remove synthetic ack path for production.

---

## 9. Live runtime service

- [x] **Single process (skeleton):** `app/runtime/live_service.py` — WS → normalize → decision → risk → signed intent → `ExecutionService` (expand features/storage).
- [ ] **Graceful shutdown:** Cancel tasks, flatten or mode-driven behavior documented.

---

## 10. Backtesting

- [ ] **Shared logic:** One code path for decision + risk vs live; automated test prevents drift.
- [ ] **Simulator:** Slippage, fees if specified, portfolio tracker; reproducible seeds.

---

## 11. Control plane

- [x] **FastAPI (partial):** Mutating routes (`POST /params`, `/system/mode`, `/flatten`) require header `X-API-Key` when `NM_CONTROL_PLANE_API_KEY` is set.
- [ ] **Streamlit:** Pages — Live, Regimes, Routes, Models, Logs, Emergency — wired to APIs/state.

---

## 12. Observability

- [ ] **Metrics:** Latency per stage, PnL, drawdown, order success, feed health; Prometheus scrape in target environment.
- [ ] **Logs:** Structured JSON to Loki (or equivalent); correlation id end-to-end.
- [ ] **Grafana:** Dashboards + alerts for stale data, disconnects, risk blocks, order failures.

---

## 13. Retraining (nightly flow)

- [ ] **Prefect (or equivalent):** Fetch data → train → evaluate → MLflow → **manual** promotion gate only.
- [ ] **No leakage:** Time-series splits; walk-forward evaluation.

---

## 14. Security & operations

- [ ] **Secrets:** Vault or env-only; rotation runbook; separate paper vs live credentials.
- [ ] **Network:** TLS; restrict control plane; optional IP allowlist for admin.
- [ ] **Runbooks:** Incident, flatten, mode change, restore from backup.

---

## 15. Testing & release

- [ ] **Integration tests:** Redis, QuestDB, Qdrant in CI (containers).
- [ ] **E2E paper:** Full loop in CI or scheduled job against paper credentials.
- [ ] **Load/soak:** WS throughput and memory bounds acceptable.
- [ ] **Release checklist:** Tag + changelog + model version pinned in deploy config.

---

## Definition of done (spec-complete)

All sections above are checked, and a short **sign-off note** is added to the release PR linking this file revision.

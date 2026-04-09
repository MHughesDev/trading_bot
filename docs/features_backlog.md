# Features backlog

**Purpose:** Things we **want to add** — capabilities, enhancements, and spec-aligned work that is not yet implemented (or only partially). This is a **planning** log, not a bug tracker.

**Related:**
- **Current bugs / fix list:** [`issue_log.md`](issue_log.md)
- **Master Spec checklist + numbered Issues 1–35:** [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md)
- **High-level gap summary:** [`MASTER_SPEC_GAPS.md`](MASTER_SPEC_GAPS.md)

Use **MASTER_SPEC_ROADMAP** as the source of truth for status checkboxes; trim or extend this file when priorities change.

---

## Data layer

| ID | Feature | Notes |
|----|---------|--------|
| FB-D1 | **Recorded WS contract tests** | Expand normalizer coverage beyond single ticker fixture; more message types (Issue 3). |
| FB-D2 | **REST candle validation** | Automated or recorded fixtures for BTC-USD, ETH-USD, SOL-USD (Issue 2). |
| FB-D3 | **L2 / order book features** | Deeper book imbalance, depth beyond current normalizers if spec requires. |
| FB-D4 | **News ingest production** | Wire `news_ingest` to real sources + dedup; align with Qdrant memory. |

## Feature pipeline (§5 Master Spec)

| ID | Feature | Notes |
|----|---------|--------|
| FB-F1 | **Full technical stack** | RSI, MACD, ATR, ADX, EMA spreads, VWAP distance per spec (Issue 9). |
| FB-F2 | **Microstructure pack** | Book imbalance, volume delta, liquidity pressure (beyond current overlays). |
| FB-F3 | **Sentiment — FinBERT** | Scores + frequency + shock features; `sentiment_use_finbert` path (Issue 9). |
| FB-F4 | **BarEvent-style contracts** | Optional strict schema (`schema_version`, `source`) end-to-end if we want parity with spec §4.3. |

## Memory (Qdrant)

| ID | Feature | Notes |
|----|---------|--------|
| FB-M1 | **Versioned embeddings** | Model/version in payload; collection schema docs (Issue 7). |
| FB-M2 | **Real 60s retrieval** | Replace placeholder memory dict with top-K + symbol filter + recency (Issue 10). |
| FB-M3 | **Memory-backed route hints** | Optional use of aggregates in route selector or action generator. |

## Models

| ID | Feature | Notes |
|----|---------|--------|
| FB-ML1 | **Trained HMM + artifacts** | Fit on historical features; persist; load only in inference (Issue 11). |
| FB-ML2 | **PyTorch TFT** | Multi-horizon returns + volatility + uncertainty; or formal deviation doc (Issue 12). |
| FB-ML3 | **MLflow training runs** | Nightly or on-demand; artifacts; registry beyond stub (Issue 14). |
| FB-ML4 | **Walk-forward evaluation** | No leakage; metrics before promotion. |

## Execution

| ID | Feature | Notes |
|----|---------|--------|
| FB-X1 | **Coinbase live adapter (complete)** | CDP/JWT signing, submit/cancel/status, idempotency (Issue 19). |
| FB-X2 | **Single execution factory** | One construction path for adapters in service + tests (Issue 17 partial acceptance). |

## Backtesting

| ID | Feature | Notes |
|----|---------|--------|
| FB-B1 | **RiskEngine + cash in replay** | Optional `available_cash` for replay-only solvency in risk (Issue 33). |
| FB-B2 | **Scenario harness** | Parameter sweeps; report templates for strategy review. |

## Control plane

| ID | Feature | Notes |
|----|---------|--------|
| FB-C1 | **Streamlit ↔ API binding** | Live, Regimes, Routes, Models, Logs, Emergency wired to FastAPI + real state (Issue 24). |
| FB-C2 | **Emergency actions UX** | Authenticated flatten / mode changes from UI. |

## Observability

| ID | Feature | Notes |
|----|---------|--------|
| FB-O1 | **Stage latency metrics** | Ingest → features → model → risk → submit (Issue 25). |
| FB-O2 | **Loki + Grafana** | JSON logs ship; dashboards for stale feed, risk blocks, order failures (Issue 26). |
| FB-O3 | **PnL / drawdown gauges** | Prometheus or derived metrics aligned with risk engine equity. |

## Orchestration & ops

| ID | Feature | Notes |
|----|---------|--------|
| FB-P1 | **Prefect nightly flow** | Data pull → train → evaluate → MLflow → manual gate (Issue 27). |
| FB-P2 | **Runbooks** | Secrets rotation, incident, flatten, QuestDB restore (Issue 28). |
| FB-P3 | **Integration CI** | Redis, QuestDB, Qdrant in CI or testcontainers (Issue 29). |
| FB-P4 | **Optional E2E paper** | Scheduled dry run with secrets (Issue 30). |
| FB-P5 | **Compose completeness** | MLflow, Prefect, Streamlit ports documented; matches README (Issue 31). |

## Infra & DX

| ID | Feature | Notes |
|----|---------|--------|
| FB-I1 | **GitHub Actions** | Lint, pytest, `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`. |
| FB-I2 | **Release checklist template** | Version tags, model version in deploy config. |

## Nice-to-haves (post–V1)

| ID | Feature | Notes |
|----|---------|--------|
| FB-N1 | Multi-exchange execution | Out of V1 scope. |
| FB-N2 | RL controllers | Spec excludes for V1. |
| FB-N3 | Portfolio optimization | Spec excludes for V1. |

---

## How to add an item

1. Add a row with a new **FB-** id (or extend the table).  
2. If it maps to Master Spec Issue *N*, note it in **Notes**.  
3. For **large** items, add or update the corresponding section in [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md).

---

*Last aligned with roadmap revision: see git history for `docs/MASTER_SPEC_ROADMAP.md`.*

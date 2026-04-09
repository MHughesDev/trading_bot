# Master Spec V3 — implementation gaps

This file records **what is not yet done** relative to the NautilusMonster V3 Master Spec (Coinbase data + Alpaca paper + shared decision/risk path). It is a **high-level gap summary** for planning and onboarding.

**Canonical backlog** (checkboxes + numbered Issues 1–35): [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md).  
**Feature ideas / fix queue:** [`features_backlog.md`](features_backlog.md) · [`issue_log.md`](issue_log.md).  
When you close work, update the roadmap first, then adjust this file if the gap list changes.

---

## Status snapshot

| Area | Gap (short) | Roadmap / issues |
|------|-------------|-------------------|
| **Execution — Coinbase live** | Signed orders (CDP/JWT), cancel, fills, idempotency | Checklist §8; [Issue 19](#issue-19-execution-coinbase-live) |
| **Models — HMM** | Train, persist, validated semantic mapping (bull/bear/volatile/sideways) | Checklist §5; [Issue 11](#issue-11-models-hmm) |
| **Models — TFT** | PyTorch TFT or documented formal deviation from spec | Checklist §5; [Issue 12](#issue-12-models-tft) |
| **Models — MLflow** | Real training runs + artifacts; registry beyond stub | Checklist §5; [Issue 14](#issue-14-mlflow) |
| **Memory — Qdrant** | Versioned payload, integration tests, real embeddings in 60s loop | Checklist §3–4; [Issues 7, 10](#issues-7-and-10-storage--memory) |
| **Features** | Full indicator set (RSI, MACD, ATR, ADX, VWAP, …), microstructure, FinBERT/sentiment | [Issue 9](#issue-9-features) |
| **Data — REST** | Live or fixture validation for BTC/ETH/SOL candles (V1 symbols) | [Issue 2](#issue-2-coinbase-rest) |
| **Storage — QuestDB** | Batching, retention, backup/runbooks | [Issue 5](#issue-5-questdb) |
| **Observability** | Loki + Grafana wiring; stage latency / PnL / order success metrics | Checklist §12; [Issues 25–26](#issues-25-26-observability) |
| **Orchestration** | Prefect nightly: data → train → evaluate → MLflow → manual gate | [Issue 27](#issue-27-prefect) |
| **Control plane UI** | Streamlit pages bound to FastAPI / live state (not shell only) | [Issue 24](#issue-24-streamlit) |
| **Ops / CI** | Integration tests (Redis, QuestDB, Qdrant); optional E2E paper; runbooks; compose completeness | [Issues 28–31](#issues-28-31-ops--ci--infra) |
| **Backtest** | Optional: pass `available_cash` into `RiskEngine` for replay | [Issue 33](#issue-33-solvency) (partial) |

---

## Checklist sections still open

From [`MASTER_SPEC_ROADMAP.md` — Production hardening checklist](MASTER_SPEC_ROADMAP.md#production-hardening-checklist):

- **§3 Storage — Qdrant:** version payload + integration tests  
- **§5 Models:** HMM train+persist; TFT or documented deviation; MLflow real runs  
- **§8 Execution:** Coinbase live signed orders  
- **§12 Observability:** Loki + Grafana deploy wiring  
- **§13–15:** Prefect flow, runbooks, integration CI  

---

## Issue cross-reference (abridged)

Use the full text and acceptance criteria in [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md#issue-log).

### Issue 2 — Coinbase REST

Pending: validate V1 symbol candles against live or recorded fixtures; partial work (retry, fallback, docs) done.

### Issues 7 and 10 — Storage / memory

- **Issue 7:** Qdrant `news_context_memory` — version payload + query tests.  
- **Issue 10:** 60s retrieval loop with **real** embeddings wired into decisions (placeholder exists in live).

### Issue 9 — Features

Microstructure + sentiment (FinBERT, frequency, shocks); full feature column set per spec §5.

### Issue 11 — Models — HMM

Train + persist Gaussian HMM; evaluation artifact path.

### Issue 12 — Models — TFT

Replace surrogate with Temporal Fusion Transformer (or formal deviation doc).

### Issue 14 — MLflow

Manual promotion policy is documented and guarded in CI; **real** orchestrated training runs still TBD.

### Issue 19 — Execution — Coinbase live

Signed orders, cancel, status, idempotency; `coinbase_live` adapter completion.

### Issue 24 — Streamlit

Pages wired to API + state (Live, Regimes, Routes, Models, Logs, Emergency).

### Issues 25–26 — Observability

Stage metrics; JSON logs to Loki; Grafana dashboards/alerts.

### Issue 27 — Prefect

Nightly retrain flow (stub in `orchestration/nightly_retrain.py`).

### Issues 28–31 — Ops / CI / infra

Secrets runbooks; integration pytest job; optional E2E paper; compose stack vs README (MLflow, Prefect, etc.).

### Issue 33 — Solvency

Replay-layer solvency implemented; optional **RiskEngine** awareness of cash in replay remains.

---

## Non-negotiables (tracking)

Spec §19 rules are largely enforced in code + CI (Coinbase-only data grep, intent signing, no raw text → trades, manual MLflow promotion guard). **Coinbase live** and **full model fidelity** are the main remaining **functional** gaps for a “complete” V1.

---

## Related docs

- [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md) — full checklist + Issues 1–35  
- [`features_backlog.md`](features_backlog.md), [`issue_log.md`](issue_log.md) — planning + operational logs  
- [`COMMENTARY.md`](COMMENTARY.md) — stub; narrative lives in roadmap  
- [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md), [`BACKTESTING_SIMULATOR.md`](BACKTESTING_SIMULATOR.md)

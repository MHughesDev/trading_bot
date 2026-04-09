# Issue log (operational)

**Purpose:** **Current issues to fix** — bugs, regressions, and incomplete work that blocks or degrades behavior. **Planned enhancements** live in [`features_backlog.md`](features_backlog.md).

---

## How to use

| Field | Meaning |
|-------|---------|
| **ID** | `IL-###` local id |
| **Severity** | Blocker / High / Medium / Low |
| **Area** | data, models, execution, risk, runtime, backtest, control_plane, observability, ops, ci |
| **Status** | Open / In progress / Done |

When resolved, move to **Resolved** or remove and reference the commit/PR.

---

## Open — system-wide

| ID | Severity | Area | Summary |
|----|----------|------|---------|
| IL-001 | High | execution | Coinbase live: real signed orders; remove `pending_implementation` synthetic acks (`execution/adapters/coinbase_live.py`). |
| IL-002 | High | models | HMM never fitted in default pipeline — regime defaults to SIDEWAYS uniform until training + load path exists. |
| IL-003 | High | models | Forecast is Ridge surrogate, not production TFT; training pipeline missing. |
| IL-004 | High | memory | Qdrant embeddings placeholder; 60s memory loop does not feed real vectors into features. |
| IL-005 | Medium | features | Sentiment / FinBERT / news pipeline returns zeros or stubs end-to-end. |
| IL-006 | Medium | storage | QuestDB: batching, retention, backup runbooks incomplete vs production needs. |
| IL-007 | Medium | observability | Loki: no Promtail (or equivalent) wiring from app logs; Grafana dashboards not as code. |
| IL-008 | High | orchestration | `nightly_flow_stub` only — no Prefect deployment or real retrain. |
| IL-009 | Medium | control_plane | Streamlit pages thin vs live API + storage. |
| IL-010 | Medium | ci | No GitHub Actions workflow; integration tests for Redis/QuestDB/Qdrant not in repo CI. |
| IL-011 | Low | runtime | `NotImplementedError` handler paths in live_service — verify coverage. |

---

## Open — data & ingest

| ID | Severity | Summary |
|----|----------|---------|
| IL-101 | Medium | `news_ingest`: `fetch_news_stub` only — no live news pipeline. |
| IL-102 | Low | Expand WS normalizer fixtures beyond current samples. |

---

## Resolved (recent)

| ID | Resolved | Summary |
|----|----------|---------|
| — | — | *Add entries when closing items.* |

---

## Quick links

- [`features_backlog.md`](features_backlog.md) — features, hardening gates, platform work  
- [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md) · [`BACKTESTING_SIMULATOR.md`](BACKTESTING_SIMULATOR.md)

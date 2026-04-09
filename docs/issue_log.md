# Issue log (operational)

**Purpose:** **Current issues to fix** — bugs, regressions, incomplete acceptance criteria, and system-wide work that blocks or degrades behavior. This complements the **features backlog** ([`features_backlog.md`](features_backlog.md)), which lists *things we want to add*.

**Authoritative numbered issues (1–35)** with full acceptance criteria: [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md#issue-log).  
**Gap summary vs spec:** [`MASTER_SPEC_GAPS.md`](MASTER_SPEC_GAPS.md).

---

## How to use

| Field | Meaning |
|-------|---------|
| **ID** | `IL-###` local id (or `Issue N` if same as roadmap) |
| **Severity** | Blocker / High / Medium / Low |
| **Area** | data, models, execution, risk, runtime, backtest, control_plane, observability, ops, ci |
| **Status** | Open / In progress / Done |

When an item is resolved, move it to **Resolved** (or delete if duplicated in roadmap) and reference the PR or commit.

---

## Open — system-wide or cross-cutting

| ID | Severity | Area | Summary | Roadmap ref |
|----|----------|------|---------|-------------|
| IL-001 | High | ops | Integration tests for Redis, QuestDB, Qdrant (containers or compose) not in CI yet. | Issue 29 |
| IL-002 | Medium | ops | Runbooks missing: secrets rotation, incident, flatten, QuestDB restore. | Issue 28 |
| IL-003 | Medium | ci | E2E paper dry run + release checklist not automated. | Issue 30 |
| IL-004 | Medium | infra | `infra/docker-compose.yml` incomplete vs spec (MLflow, Prefect, Streamlit ports/docs). | Issue 31 |
| IL-005 | High | execution | Coinbase **live** adapter: signed orders, cancel, fills, idempotency — `pending_implementation` path. | Issue 19 |
| IL-006 | High | models | HMM and TFT still stubs / surrogates; no persisted training pipeline. | Issues 11–12 |
| IL-007 | High | memory | Qdrant: no versioned payload + integration tests; 60s loop still placeholder embeddings. | Issues 7, 10 |
| IL-008 | Medium | features | Full indicator + sentiment stack (FinBERT path) incomplete vs Master Spec §5. | Issue 9 |
| IL-009 | Medium | data | Coinbase REST: V1 symbol candle validation vs live or recorded fixtures still open. | Issue 2 |
| IL-010 | Medium | storage | QuestDB: batching, retention, backup runbook — partial only. | Issue 5 |
| IL-011 | Medium | observability | Loki + Grafana not wired; stage latency / PnL metrics incomplete. | Issues 25–26 |
| IL-012 | High | orchestration | Prefect nightly retrain is stub only (`orchestration/nightly_retrain.py`). | Issue 27 |
| IL-013 | Medium | mlflow | Real MLflow training runs from orchestration; registry workflow beyond no-op `promote()`. | Issue 14 |
| IL-014 | Medium | control_plane | Streamlit pages are shells; not fully bound to API + live state. | Issue 24 |
| IL-015 | Low | runtime | Graceful shutdown: signal handling not covered in CI. | Issue 21 |
| IL-016 | Low | backtest | Optional: `RiskEngine` sees `available_cash` in replay for stricter solvency. | Issue 33 |
| IL-017 | Medium | decision | RouteDecision ranking + full action-vs-risk matrix tests incomplete. | Issue 15 |
| IL-018 | Low | execution | Alpaca: optional CI integration test against paper API (secrets). | Issue 18 |
| IL-019 | Medium | runtime | “Full” live pipeline + QuestDB audit — partial; verify all paths under load. | Issue 20 |

---

## Open — data & ingestion

| ID | Severity | Summary | Roadmap ref |
|----|----------|---------|-------------|
| IL-101 | Medium | Normalizer tests: expand recorded Coinbase WS payloads beyond current fixture. | Issue 3 |
| IL-102 | Low | Epic tracker remains **Pending** until full spec compliance. | Epic in roadmap |

---

## Open — risk & backtest

| ID | Severity | Summary | Roadmap ref |
|----|----------|---------|-------------|
| IL-201 | Low | Issue 16: epic acceptance still marked Pending in roadmap despite many checkboxes done — reconcile doc vs code. | Issue 16 |

---

## Resolved (recent)

| ID | Resolved | Summary |
|----|----------|---------|
| — | — | *Add entries when closing IL items (e.g. IL-xxx fixed in commit abc1234).* |

---

## Quick links

- [`features_backlog.md`](features_backlog.md) — desired features and enhancements  
- [`MASTER_SPEC_ROADMAP.md`](MASTER_SPEC_ROADMAP.md) — full Issue 1–35 bodies  
- [`MASTER_SPEC_GAPS.md`](MASTER_SPEC_GAPS.md) — condensed gap table  
- [`GITHUB_ISSUES.md`](GITHUB_ISSUES.md) — optional sync to GitHub Issues  

---

*Operational log: not a substitute for the numbered issue definitions in MASTER_SPEC_ROADMAP.md.*

# Issue log

**Purpose:** A single place for **things we need to fix** on **existing** functions and components — bugs, incorrect behavior, stubs that break contracts, missing wiring in code paths that already exist, and operational/debt items that are **fixes**, not new capabilities.

**Not here:** new features, greenfield work, or release gates — use **[`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD)** only. No duplicate line items between this file and the backlog.

---

## Conventions

| Field | Meaning |
|-------|---------|
| **ID** | `IL-###` (local) |
| **Severity** | Blocker / High / Medium / Low |
| **Area** | data, models, execution, risk, runtime, backtest, control_plane, observability, ops, ci |
| **Status** | Open / In progress / Done |

When you close an item: set **Status** to **Done**, add **Resolved** date + commit/PR in **Resolved (recent)**, or remove the row if you prefer a minimal open list.

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
| IL-011 | Low | runtime | `NotImplementedError` handler paths in `live_service` — verify coverage. |

---

## Open — data & ingest

| ID | Severity | Summary |
|----|----------|---------|
| IL-101 | Medium | `news_ingest`: `fetch_news_stub` only — no live news pipeline. |
| IL-102 | Low | Expand WS normalizer fixtures beyond current samples. |

---

## Syncing to GitHub Issues (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift from this log; prefer **updating this file** for operational fixes, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; this markdown file remains the **local** source of truth for fix tracking.

---

## Resolved (recent)

| ID | Resolved | Summary |
|----|----------|---------|
| — | — | *Add entries when closing items.* |

---

## Related

- [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) — planned work only (no overlap with rows above)  
- [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)

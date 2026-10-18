# Queue — agent protocol & conventions

**Documentation last reviewed:** **2026-04-18** (`queue_top.sh` / `queue_close.sh` workflow).

**Queue system:** The **queue system** is every file that implements backlog machinery — see **[`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md)** for the full list (`QUEUE_STACK.csv`, `QUEUE_ARCHIVE.MD`, `AUTOMATION_QUEUE_SLICE_PROMPT.MD`, **Add to Queue** skill, optional `scripts/generate_queue_stack.py`). This file (`QUEUE.MD`) holds **protocol and conventions** only.

---

## Agent protocol (read first — token-efficient)

0. **Every agent session:** Read **[`README.md`](../README.md)** and **[`AGENTS.md`](../AGENTS.md)** in full first — **re-read `AGENTS.md`** after any context reset or new thread. Then use the commands below for task selection (steps 1–4).
1. **Do not** read this whole file to pick work. **Do not** open **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** or **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** in full. From repo root run **`bash scripts/queue_top.sh`** (same as **`python3 scripts/print_next_queue_item.py`**) to print the full next **`Open`** row to stdout (optional **`--json`** — when the backlog is empty, JSON mode prints **`{"queue_empty": true, "message": "..."}`**).
2. The script selects the smallest **`stack_order`** with **`status`** = **`Open`** (excluding **`_QUEUE_EMPTY_`**). If output is **`QUEUE_EMPTY:`** or **`id`** = **`_QUEUE_EMPTY_`** → **stop** (nothing to do).
3. **Implement from that one row:** use **`agent_task`**, **`affected_files`**, optional **`docs_refs`** (read only those paths if present). You do **not** need other markdown files to understand the task.
4. After completing work: run **`bash scripts/queue_close.sh --next`** (or **`bash scripts/queue_close.sh --id <ID>`**) to set **`status=Done`** in [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py), regenerate **`QUEUE_STACK.csv`**, and update a matching **`Open` → `Done`** row in **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** when the ID appears in a standard pipe table. Then refresh **[§2](#2-open-queue-status)** counts in this file if needed and follow **[§6](#6-how-to-add-or-close-an-item)**. **Do not merge the PR to `main`** (e.g. `gh pr merge`) until this closure is **committed and pushed**. See [`AGENTS.md`](../AGENTS.md) §7 and [`AUTOMATION_QUEUE_SLICE_PROMPT.MD`](AUTOMATION_QUEUE_SLICE_PROMPT.MD) Phase 6.

Step **0** is the repository contract; steps **1–4** are the minimal path to pick and execute the next queue item without loading this whole file each time.

**Human / history:** Full tables (completed epics, `IL-*`, archive) live in **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)**. Cross-repo note: copy **`QUEUE.MD`** + **`QUEUE_STACK.csv`** schema together; see §0 column definitions.

**References (background):** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`AUDIT_CODE_REVIEW.MD`](AUDIT_CODE_REVIEW.MD) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

---

## Contents

| § | Section |
|---|---------|
| [§0](#0-next-task-stack-queue_stackcsv) | **Next task stack** (`QUEUE_STACK.csv` — **canonical for agents**) |
| [§1](#1-conventions) | Conventions (Kind, IDs, priority, phase, category, batch) |
| [§2](#2-open-queue-status) | **Open queue status** (snapshot + pointer to archive) |
| [§3](#3-historical-tables) | Historical tables (`QUEUE_ARCHIVE.MD`) |
| [§4](#4-what-is-already-in-good-shape) | Healthy areas |
| [§5](#5-github-sync-optional) | GitHub sync (optional) |
| [§6](#6-how-to-add-or-close-an-item) | How to add or close an item |

| Related (queue system) | Purpose |
|---------|---------|
| [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) | **Index of all queue-system files** (this repo) |
| [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) | Full §2–§5 tables (open detail, resolved fixes, gates, archive) |
| [`AUTOMATION_QUEUE_SLICE_PROMPT.MD`](AUTOMATION_QUEUE_SLICE_PROMPT.MD) | PR / merge workflow for one slice |
| [`.cursor/skills/add-to-queue/SKILL.md`](../.cursor/skills/add-to-queue/SKILL.md) | **Add to Queue** skill |
| [`.cursor/skills/queue-one-at-a-time/SKILL.md`](../.cursor/skills/queue-one-at-a-time/SKILL.md) | **Queue One-at-a-Time** execution skill |
| [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py) | Optional CSV regenerator |

---

## 0. Next task stack (`QUEUE_STACK.csv`)

**File:** [`QUEUE_STACK.csv`](QUEUE_STACK.csv) — UTF-8 CSV. **One row = one actionable task** with enough detail that an agent rarely needs other docs.

| Column | Required | Meaning |
|--------|------------|---------|
| **`stack_order`** | yes | Integer — **1** = top. Sort **ascending**; lowest `stack_order` with `status=Open` is next. **Maintainers:** if you use [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py), **do not** store `stack_order` in the `ROWS` dicts — **list order** is the stack; the script fills `stack_order` when writing the CSV. |
| **`priority`** | yes | **HIGH** / **MEDIUM** / **LOW** — filter for “next HIGH” workflows; **pull order** is **`stack_order`**, not priority alone. |
| **`phase`** | yes | **A** / **B** / **C** / **D** (see [§1.3](#13-priority-phase-category-status)). |
| **`batch`** | no | Theme code (e.g. **AUD-OPS**, **AP-PAM**) — grouping only. |
| **`id`** | yes | Queue ID (e.g. **FB-AUD-003**) or **`_QUEUE_EMPTY_`** when no Open work. |
| **`kind`** | yes | **Kind** from [§1.1](#11-kind-work-type) — e.g. `platform`, `tech_debt`, `hardening`. |
| **`status`** | yes | **`Open`** / **`In progress`** / **`Done`** / **`empty`** (only with **`_QUEUE_EMPTY_`**). |
| **`summary`** | yes | Short execution summary (1–2 clauses) used by agents and status snapshots; should align with `summary_one_line`. |
| **`summary_one_line`** | yes | Short title for humans and grep. |
| **`agent_task`** | yes | **Self-sufficient instructions:** goal, acceptance criteria, and constraints in plain text (can be long; use `\|` or newlines escaped as needed in CSV). |
| **`affected_files`** | no | Pipe-separated **primary** paths or globs — **starting points**, not exhaustive. |
| **`docs_refs`** | no | Pipe-separated **optional** docs for context (e.g. `docs/RUNBOOKS.MD`). |
| **`audit_id`** | no | Traceability (e.g. **OPS-A1**, **DOC-A1**) when promoted from an audit. |
| **`anchor`** | no | Legacy anchor into `QUEUE_ARCHIVE.MD` (e.g. `#25-post-audit-hardening-fb-aud`). |

**Sentinel row when backlog is empty:**

```text
stack_order,priority,phase,batch,id,kind,status,summary,summary_one_line,agent_task,affected_files,docs_refs,audit_id,anchor
9999,LOW,D,,_QUEUE_EMPTY_,deferred,empty,No open queue items,No open queue items,N/A — add work via §6.,,,,
```

**Maintenance:** Any PR that adds/closes/reorders **Open** work must update **`QUEUE_STACK.csv`** and mirror status lines into **`QUEUE_ARCHIVE.MD`** where that ID has a narrative row. After editing **`scripts/generate_queue_stack.py`** `ROWS`, run **`python scripts/generate_queue_stack.py`** so the CSV matches (numbers are derived, not hand-edited in Python).

---

## 1. Conventions

### 1.1 Kind (work type)

| Kind | Meaning |
|------|--------|
| **fix** | Wrong behavior, bug, contract break (`IL-*`). |
| **feature** | New user- or operator-visible capability. |
| **change** | Improvement — not a bug, not a whole new surface. |
| **hardening** | Security, signing, guardrails. |
| **platform** | Infra, CI, compose, packaging. |
| **reliability** | Ops: runbooks, smokes. |
| **spike** | Time-boxed research. |
| **tech_debt** | Refactors. |
| **compliance** | Release gates (`HG-*`). |
| **deferred** | Out of scope for now (`FB-N*`). |

### 1.2 ID prefixes

| Prefix | Scope |
|--------|--------|
| **FB-** | Roadmap, epics, deferred |
| **IL-** | Fixes only |
| **HG-** | Compliance gates |

### 1.3 Priority, phase, category, status

**Priority:** HIGH / MEDIUM / LOW (maps to P0–P3).

**Phase:** **A** — core trading intelligence; **B** — operator product; **C** — distributed platform; **D** — deferred.

**Batch tables** (AP-*, UX-*, CONT-*, AUD-*): unchanged — see **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** header sections or §1 in archive copy.

**Category:** **FR**, **PF**, **UI**, **DS**, **MS**, **XX** (roadmap); **EX**, **MD**, **DP**, **RT**, **CP**, **OB**, **OP**, **CI**, **CFG** (fixes).

**Status:** Open / In progress / Done / Superseded. A **Superseded** row (added 2026-10-03) targets code that no longer exists; its `agent_task` names the successor and keeps the original text for the record.

---

## 2. Open queue status

**Snapshot (2026-10-03).** The Python tree this queue was written against (`control_plane/`, `execution/adapters/`, `forecaster_model/`, `backtesting/`) was deleted in the Rust rewrite. The 2026-06-08 open stack has been triaged against the current Rust/React stack:

| ID | Status | Successor / current form |
|----|--------|--------------------------|
| FB-FCST-001 | **Superseded** | Set I (probabilistic forecasting suite) + Set U Phase 5 (typed model contracts) |
| FB-FCST-002 | **Superseded** | Set J/K backtest suite, run through Set U Phase 3 with Set U Phase 5 model inputs |
| FB-RX-001 | **Superseded** | COMP-007 live monitor (`crates/monitor`, `/api/monitor/*`), Set S Phase 7 / Set T |
| FB-EXEC-001 | **Superseded** | ADR-0048: paper runs in the app; the in-app paper engine already fills limit/stop (`crates/execution/src/paper/clob.rs`) |
| FB-EXEC-002 | **Superseded** | The live Coinbase adapter (`crates/execution/src/coinbase.rs`, a stub), gated by ROADMAP H-4; see FB-N1-S2 |
| FB-RX-002 | Open, **rewritten** | Live bars on the chart over the existing `/ws/live` WebSocket |
| FB-RX-003 | Open, **rewritten** | Execution-quality view from COMP-007 slippage stats + venue health |
| FB-RX-004 | Open, **rewritten** | Governance view over gates, platform timeline, model-alias rollback, deployments |
| FB-RX-005 | Open, **rewritten** | Alert inbox over `/api/monitor/events` + `/api/alerts`, ack persisted server-side |
| FB-N1-S2 | Open, **rewritten** (carries FB-N1) | Kraken spot live adapter behind the `Venue` port + per-venue risk caps; H-4 gated |
| FB-N3-S2 | Open, **new** (carries FB-N3) | Optimiser objectives on `strategy_engine::allocation::rebalance` |

Full rewritten task text is in [`QUEUE_STACK.csv`](QUEUE_STACK.csv) rows 121–128. These rows are a holding pen: the programme is tracked in [`plans/PLAN.md`](plans/PLAN.md), and a track that takes one of them should move it into a plan set.

| Priority | Open count | IDs |
|----------|------------|-----|
| **HIGH** | **1** | FB-RX-002 |
| **MEDIUM** | **2** | FB-RX-003, FB-RX-004 |
| **LOW** | **3** | FB-RX-005, FB-N1-S2, FB-N3-S2 |

*Superseded on 2026-10-03:* FB-FCST-001, FB-FCST-002, FB-RX-001, FB-EXEC-001, FB-EXEC-002. *The original 2026-06-08 snapshot read:* open stack FB-FCST-001 → FB-FCST-002 → FB-RX-001 → FB-RX-002 → FB-RX-003 → FB-RX-004 → FB-RX-005 → FB-EXEC-001 → FB-EXEC-002 → FB-N1-S2.

**Full row text and completed history:** [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) (§2 open tables, §3 fixes, §4 gates, §5 archive).

---

## 3. Historical tables

All detailed tables moved to **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** to keep agent context small. IDs and outcomes are unchanged from the pre-split monolithic **`QUEUE.MD`**.

**Deep links (same anchors as before):** e.g. [§2.5 post-audit FB-AUD](QUEUE_ARCHIVE.MD#25-post-audit-hardening-fb-aud) · [§2.2 FB-AP-P0](QUEUE_ARCHIVE.MD#22-per-asset-product-epic-fb-ap-p0) · [§2.3 FB-CONT-P0](QUEUE_ARCHIVE.MD#23-container--oci-cloud-deployment-fb-cont-p0) — update bookmarks that pointed at `QUEUE.MD#...` to `QUEUE_ARCHIVE.MD#...`.

---

## 4. What is already in good shape

| Area | Notes |
|------|-------|
| **Decision** | Shared **`run_decision_tick`**; **`replay_decisions`** / **`replay_multi_asset_decisions`**. |
| **Execution** | **Alpaca paper** adapter; **Coinbase live** execution adapter (orders only). **Market data:** Kraken REST + WebSocket. |
| **Features** | **Feature pipeline** (RSI, MACD, ATR, ADX, VWAP, microstructure). |
| **Risk** | **Risk** + HMAC + CI guards. |
| **Control / observability** | **FastAPI** + **Prometheus**; **Compose** core services. |

---

## 5. GitHub sync (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift; prefer updating **`QUEUE_ARCHIVE.MD`** / **`QUEUE_STACK.csv`** for operational truth, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; **`QUEUE_STACK.csv`** + archive remain canonical locally.

---

## 6. How to add or close an item

1. Use the **[Add to Queue](../.cursor/skills/add-to-queue/SKILL.md)** skill or follow the same steps manually.
2. **Assign** Kind, priority, phase, batch (if any), unique **id**, and **status**.
3. **Append a row to [`QUEUE_STACK.csv`](QUEUE_STACK.csv)** with a non-empty **`agent_task`** (self-sufficient) and **`affected_files`** / **`docs_refs`** as needed — **or** append a dict to **`ROWS`** in [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py) and run the script (preferred when bulk-updating history: **no per-row renumbering** in Python).
4. **Set pull order:** either assign **`stack_order`** in the CSV so the next **`Open`** row has the **smallest** number among Open rows, **or** (when using the generator) **move the dict** earlier/later in **`ROWS`** — numbers are filled automatically. Remove **`_QUEUE_EMPTY_`** when adding Open work; restore the sentinel row when the backlog becomes empty (**last** row in **`ROWS`** must stay **`_QUEUE_EMPTY_`** when using the script).
5. **Mirror** the same summary row into **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** §2 (or the right subsection) so humans can read narrative tables — **keep CSV and archive in sync**.
6. On **close:** set **`status=Done`**, optionally remove the row after a release, and update the archive table.

**Empty backlog:** If no **`Open`** rows, agents **stop** — add work via this section or reopen items in the archive.

---

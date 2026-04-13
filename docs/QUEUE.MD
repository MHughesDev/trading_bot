# Queue — agent protocol & conventions

**Documentation last reviewed:** **2026-04-13**.

**Queue system:** The **queue system** is every file that implements backlog machinery — see **[`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md)** for the full list (`QUEUE_STACK.csv`, `QUEUE_ARCHIVE.MD`, `AUTOMATION_QUEUE_SLICE_PROMPT.MD`, **Add to Queue** skill, optional `scripts/generate_queue_stack.py`). This file (`QUEUE.MD`) holds **protocol and conventions** only.

---

## Agent protocol (read first — token-efficient)

1. **Do not** read this whole file to pick work. Open **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** only.
2. Sort by **`stack_order`** ascending. The **next task** is the first row where **`status`** = `Open`. If **`id`** = `_QUEUE_EMPTY_` or there is no `Open` row → **stop** (nothing to do).
3. **Implement from one CSV row:** use **`agent_task`** (full instructions), **`affected_files`** (where to start), **`docs_refs`** (optional deeper context). You do **not** need other markdown files to understand the task.
4. After completing work: set **`status`** to `Done` in the CSV (or remove the row), adjust **`stack_order`** for remaining Open items, mirror the status into **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** §2.5 if that ID exists there, and follow **[§6](#6-how-to-add-or-close-an-item)**.

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
| [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py) | Optional CSV regenerator |

---

## 0. Next task stack (`QUEUE_STACK.csv`)

**File:** [`QUEUE_STACK.csv`](QUEUE_STACK.csv) — UTF-8 CSV. **One row = one actionable task** with enough detail that an agent rarely needs other docs.

| Column | Required | Meaning |
|--------|------------|---------|
| **`stack_order`** | yes | Integer — **1** = top. Sort **ascending**; lowest `stack_order` with `status=Open` is next. |
| **`priority`** | yes | **HIGH** / **MEDIUM** / **LOW** — filter for “next HIGH” workflows; **pull order** is **`stack_order`**, not priority alone. |
| **`phase`** | yes | **A** / **B** / **C** / **D** (see [§1.3](#13-priority-phase-category-status)). |
| **`batch`** | no | Theme code (e.g. **AUD-OPS**, **AP-PAM**) — grouping only. |
| **`id`** | yes | Queue ID (e.g. **FB-AUD-003**) or **`_QUEUE_EMPTY_`** when no Open work. |
| **`kind`** | yes | **Kind** from [§1.1](#11-kind-work-type) — e.g. `platform`, `tech_debt`, `hardening`. |
| **`status`** | yes | **`Open`** / **`In progress`** / **`Done`** / **`empty`** (only with **`_QUEUE_EMPTY_`**). |
| **`summary_one_line`** | yes | Short title for humans and grep. |
| **`agent_task`** | yes | **Self-sufficient instructions:** goal, acceptance criteria, and constraints in plain text (can be long; use `\|` or newlines escaped as needed in CSV). |
| **`affected_files`** | no | Pipe-separated **primary** paths or globs — **starting points**, not exhaustive. |
| **`docs_refs`** | no | Pipe-separated **optional** docs for context (e.g. `docs/RUNBOOKS.MD`). |
| **`audit_id`** | no | Traceability (e.g. **OPS-A1**, **DOC-A1**) when promoted from an audit. |
| **`anchor`** | no | Legacy anchor into `QUEUE_ARCHIVE.MD` (e.g. `#25-post-audit-hardening-fb-aud`). |

**Sentinel row when backlog is empty:**

```text
stack_order,priority,phase,batch,id,kind,status,summary_one_line,agent_task,affected_files,docs_refs,audit_id,anchor
9999,LOW,D,,_QUEUE_EMPTY_,deferred,empty,No open queue items,N/A — add work via §6.,,,,
```

**Maintenance:** Any PR that adds/closes/reorders **Open** work must update **`QUEUE_STACK.csv`** and mirror status lines into **`QUEUE_ARCHIVE.MD`** where that ID has a narrative row.

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

**Status:** Open / In progress / Done.

---

## 2. Open queue status

**Snapshot (2026-04-16):** Roadmap epics **FB-AP-P0**, **FB-UX-P0**, **FB-CONT-P0** — **Done**. **FB-DOC-024** (README Quick start first) — **Done**. **FB-SETUP-021**…**023** — **Done**. **FB-UX-015** / **FB-UX-016** — **Done**. **FB-RUN-018**…**020** — **Done**. **FB-AUD-001**…**020** and **FB-CONT-009** — **Done**. **Open stack:** empty — sentinel **`_QUEUE_EMPTY_`** in **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)**.

| Priority | Open count | IDs |
|----------|------------|-----|
| **HIGH** | **0** | — |
| **MEDIUM** | **0** | — |
| **LOW** | **0** | — |

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
3. **Append a row to [`QUEUE_STACK.csv`](QUEUE_STACK.csv)** with a non-empty **`agent_task`** (self-sufficient) and **`affected_files`** / **`docs_refs`** as needed.
4. **Set `stack_order`** so the next **`Open`** item has the **smallest** number among Open rows (renumber if needed). Remove **`_QUEUE_EMPTY_`** when adding Open work; insert the sentinel row when the backlog becomes empty.
5. **Mirror** the same summary row into **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** §2 (or the right subsection) so humans can read narrative tables — **keep CSV and archive in sync**.
6. On **close:** set **`status=Done`**, optionally remove the row after a release, and update the archive table.

**Empty backlog:** If no **`Open`** rows, agents **stop** — add work via this section or reopen items in the archive.

---

---
name: add-to-queue
description: Add or update work items in the queue system (QUEUE_STACK.csv, QUEUE_ARCHIVE.MD, QUEUE.MD per docs/QUEUE_SCHEMA.md). Use when the user asks to log backlog items, queue tasks, or record fixes/features in the queue.
---

# Add to Queue

**Queue system:** Follow [`docs/QUEUE_SCHEMA.md`](../../../docs/QUEUE_SCHEMA.md) — this skill touches **`QUEUE_STACK.csv`**, **`QUEUE_ARCHIVE.MD`**, and the **`QUEUE.MD`** snapshot.

Follow this workflow whenever you need to **append or edit** the project work queue.

## Source of truth (queue system files)

| Artifact | Role |
|----------|------|
| **[`docs/QUEUE.MD`](../../../docs/QUEUE.MD)** | **Conventions** (Kind, IDs, batch codes), **agent protocol**, and **how to add/close** items — small file; agents read this for rules only. |
| **[`docs/QUEUE_STACK.csv`](../../../docs/QUEUE_STACK.csv)** | **Next-task stack** — **one row = one actionable task**. Required columns include **`agent_task`** (self-sufficient instructions), **`affected_files`**, **`docs_refs`**. |
| **[`docs/QUEUE_ARCHIVE.MD`](../../../docs/QUEUE_ARCHIVE.MD)** | **Human-readable tables** — open-queue detail, `IL-*`, `HG-*`, completed archive. **Mirror** new/changed **Open** rows here when you add narrative tables. |

**Legacy:** The monolithic `QUEUE.MD` was split (2026) — full tables live in **`QUEUE_ARCHIVE.MD`**.

## Before you edit

1. **Read** [§1 Conventions](../../../docs/QUEUE.MD#1-conventions) and [§0 CSV columns](../../../docs/QUEUE.MD#0-next-task-stack-queue_stackcsv) in `QUEUE.MD`.
2. **Decide the target section in the archive** (if mirroring):
   - **New open roadmap item** → `QUEUE_ARCHIVE.MD` §2 (correct subsection / epic table).
   - **Resolved fix** → §3 (`IL-*` only).
   - **Completed gate** → §4.
   - **Completed theme / archive row** → §5.
3. **Pick the next ID** — do not collide with existing IDs; prefer sub-IDs (`FB-XXX-01`) for slices under an epic.
4. **`agent_task` must stand alone** — an agent implementing the row should not *need* another `.md` file. Optional **`docs_refs`** point to background only.

## Row shape (`QUEUE_STACK.csv`)

Required headers include: `stack_order`, `priority`, `phase`, `batch`, `id`, `kind`, `status`, `summary`, `summary_one_line`, `agent_task`, `affected_files`, `docs_refs`, `audit_id` (optional), `anchor` (optional).

- **`agent_task`:** Goal, acceptance criteria, and constraints in plain text (quote the field if it contains commas).
- **`stack_order`:** Smallest number among **`Open`** rows = next task. **If** you use [`scripts/generate_queue_stack.py`](../../../scripts/generate_queue_stack.py): reorder or append entries in **`ROWS`** (sentinel **`_QUEUE_EMPTY_`** last), run the script — **do not** edit numeric **`stack_order`** in the Python file. **If** you edit **`QUEUE_STACK.csv`** directly, set **`stack_order`** on each row.
- **`affected_files`:** pipe-separated primary paths (`control_plane/api.py|tests/`).

## After editing

1. **`QUEUE_STACK.csv`** — row added/updated; **`_QUEUE_EMPTY_`** sentinel removed when adding Open work, restored when backlog empty.
2. **`QUEUE_ARCHIVE.MD`** — same ID row updated in §2 (or appropriate section) so narrative history stays aligned.
3. **`QUEUE.MD` §2 snapshot** — update the small priority snapshot table if Open counts/IDs change.
4. Optional: **`scripts/generate_queue_stack.py`** — after editing **`ROWS`**, run **`python scripts/generate_queue_stack.py`** to refresh **`QUEUE_STACK.csv`** (numbers derived from list order).

## Checklist

- [ ] **`agent_task`** is self-sufficient (no mandatory external doc read).
- [ ] **`kind`** matches work type; **`IL-*`** only for fixes in archive §3.
- [ ] **`status`** set (`Open` / `In progress` / `Done`).
- [ ] Pull order is correct (**`stack_order`** in CSV, or **`ROWS`** order + regenerate script).
- [ ] **`QUEUE_ARCHIVE.MD`** mirrored for Open items that have a §2 table row.
- [ ] **`QUEUE.MD` §2** snapshot updated if Open set changed.

## When not to use this skill

- **Implementing** queue items (code changes) — use normal development workflow + tests in `AGENTS.md`.
- **Renaming** queue files — requires explicit repo decision and bulk link updates.

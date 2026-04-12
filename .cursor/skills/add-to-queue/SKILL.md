---
name: add-to-queue
description: Add or update work items in docs/QUEUE.MD using project conventions (Kind, IDs, Batch, Affected files). Use when the user asks to log backlog items, queue tasks, or record fixes/features in the queue.
---

# Add to Queue

Follow this workflow whenever you need to **append or edit** the project work queue. The **canonical file** is **`docs/QUEUE.MD`** (uppercase `.MD`). The legacy name **QueueLog** refers to the same document—**do not** create a separate `QueueLog.md` unless the user explicitly asks to rename the file.

## Source of truth

- **File:** [`docs/QUEUE.MD`](../../../docs/QUEUE.MD) — single backlog + resolved history (full narrative).
- **Next-task stack (agents read first):** [`docs/QUEUE_STACK.csv`](../../../docs/QUEUE_STACK.csv) — ordered **`stack_order`**; **`Open`** rows = backlog top to bottom. Update **`QUEUE_STACK.csv`** whenever you add/close **Open** items in **`QUEUE.MD`** (see **`QUEUE.MD` §0**).
- **Automation / PR workflow:** [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](../../../docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD).
- **Agent boundaries:** [`AGENTS.md`](../../../AGENTS.md) (Kraken-only market data, risk/signing, no secrets, no MLflow auto-promotion in code).

## Before you edit

1. **Read** [§1 Conventions](../../../docs/QUEUE.MD#1-conventions) in `QUEUE.MD` — **Kind**, **ID prefixes** (`FB-`, `IL-`, `HG-`), **Priority**, **Phase**, **Category**, **Batch** (for `FB-AP-*`), **Status**.
2. **Decide the target section:**
   - **New open roadmap item** → [§2 Open queue](../../../docs/QUEUE.MD#2-open-queue) (correct subsection: HIGH/MEDIUM/LOW, epic table, or deferred §2.1).
   - **Resolved fix** → [§3 Resolved fixes](../../../docs/QUEUE.MD#3-resolved-fixes-il) (`IL-*` only).
   - **Completed gate** → [§4](../../../docs/QUEUE.MD#4-completed-compliance-gates-hg).
   - **Completed theme / archive row** → [§5](../../../docs/QUEUE.MD#5-completed-archive-fb--themes).
3. **Pick the next ID** — do not collide with existing IDs; prefer sub-IDs (`FB-XXX-01`) for slices under an epic.
4. **Do not** duplicate long narratives across files; **link** to `docs/*.MD` from the Summary when helpful.

## Row shape (FB-AP-P0 style table)

When adding rows to the **FB-AP-P0** epic table, use columns:

`ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files`

- **Summary:** one cell, **as detailed as needed** (requirements, pointers, acceptance hints)—long summaries are encouraged for implementers.
- **Affected files:** pipe-separated **primary** paths or globs (`control_plane/api.py`, `tests/`, …).
- **Batch:** use the [Batch lookup table](../../../docs/QUEUE.MD#13-priority-phase-category-status) (e.g. `AP-PAM`, `AP-INI`).

## Row shape (fixes `IL-*`)

Use the subsection template in §3: **Kind**, **Phase** (P1/P2/P3), **Resolved** date, **Summary** with code paths.

## After editing the queue

1. **Stack CSV:** Update [`QUEUE_STACK.csv`](../../../docs/QUEUE_STACK.csv) — keep **`stack_order`** monotonic with intended pull order; set finished rows to **`Done`** or remove them; use **`_QUEUE_EMPTY_`** / **`empty`** when there are no **`Open`** rows (see **`QUEUE.MD` §0**).
2. **Consistency:** If behavior or operator flows changed elsewhere, update **README** or the relevant **`docs/*.MD`** per `AGENTS.md`—but **do not** expand scope beyond what the user asked.
3. **Optional GitHub sync:** §7 of `QUEUE.MD` — mirroring to GitHub issues is optional; markdown stays canonical.

## Checklist

- [ ] Correct **§** (open vs resolved vs archive).
- [ ] **Kind** matches work type; **`IL-*`** only for fixes.
- [ ] **Status** set (`Open` / `In progress` / `Done` / `deferred`).
- [ ] **Summary** names verify steps or files where possible.
- [ ] **Contents** table at top of `QUEUE.MD` updated if you add a **new § anchor** (rare).
- [ ] **`QUEUE_STACK.csv`** updated if **`Open`** / stack order changed.

## When not to use this skill

- **Implementing** queue items (code changes) — use normal development workflow + tests in `AGENTS.md`.
- **Renaming** `QUEUE.MD` — requires explicit repo decision and bulk link updates.

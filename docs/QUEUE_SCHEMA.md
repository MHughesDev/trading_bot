# Queue system — portable schema (any repository)

**Documentation last reviewed:** **2026-02-06** (full doc pass: terminology, queue-system scope, cross-links).

---

## What “queue system” means in this repo

The **queue system** is **all artifacts that define, store, and operate the work backlog machinery** — not only `QUEUE.MD`. When editing backlog **process**, **schema**, or **next-task** behavior, touch every relevant file below so nothing drifts.

| Artifact | Role |
|----------|------|
| [`QUEUE.MD`](QUEUE.MD) | **Agent protocol** + **conventions** (Kind, IDs, how to add/close items). Small file; read for rules, not full history. |
| [`QUEUE_STACK.csv`](QUEUE_STACK.csv) | **Next-task stack** — machine-readable; **`agent_task`** per row; canonical for **which** task runs next. |
| [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) | **Narrative tables** — open detail, `IL-*`, `HG-*`, completed `FB-*` archive. |
| [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) | **This file** — portable schema + queue-system index. |
| [`AUTOMATION_QUEUE_SLICE_PROMPT.MD`](AUTOMATION_QUEUE_SLICE_PROMPT.MD) | Agent workflow: one slice → validate → PR → merge. |
| [`.cursor/skills/add-to-queue/SKILL.md`](../.cursor/skills/add-to-queue/SKILL.md) | Cursor **Add to Queue** skill for adding/updating items. |
| [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py) | Optional CSV **regenerator** (maintainer tool; edit `ROWS` and run). |

**Audit → backlog (optional):** [`docs/FULL_AUDIT.md`](FULL_AUDIT.md) **§8** audit report · [`.cursor/skills/draft-audit-report`](../.cursor/skills/draft-audit-report/SKILL.md) · [`.cursor/skills/audit-report-to-queue`](../.cursor/skills/audit-report-to-queue/SKILL.md) · [`docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD`](BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD)

**Related (not part of the core queue system but often updated together):** [`scripts/create_github_issues.sh`](../scripts/create_github_issues.sh) (optional GitHub mirror).

---

## Copying to another repository

Copy **`QUEUE.MD`** + **`QUEUE_STACK.csv`** together at minimum; add **`QUEUE_ARCHIVE.MD`** when you need full tables. See column definitions in [`QUEUE.MD` §0](QUEUE.MD#0-next-task-stack-queue_stackcsv).

**Minimum viable:** `QUEUE.MD` + `QUEUE_STACK.csv` only; fold archive tables into `QUEUE.MD` if you want a single file (higher token cost for agents).

**Regenerator:** maintain `scripts/generate_queue_stack.py` or edit CSV by hand.

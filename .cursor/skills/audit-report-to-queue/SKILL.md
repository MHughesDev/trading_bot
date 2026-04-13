---
name: audit-report-to-queue
description: Promote Open findings from a standalone audit report (FULL_AUDIT §8 format) into QUEUE_STACK.csv and mirror QUEUE_ARCHIVE.MD per QUEUE_SCHEMA.md. Use after draft-audit-report or an existing AUDIT_REPORT_*.md.
---

# Audit report → queue stack

**Purpose:** Convert **Open** rows in the **## Findings** table of an audit report into **`QUEUE_STACK.csv`** rows with **self-contained `agent_task`** text, following the **[queue system](../../../docs/QUEUE_SCHEMA.md)**.

## Prerequisites

1. Read **[`docs/QUEUE_SCHEMA.md`](../../../docs/QUEUE_SCHEMA.md)** and **[`docs/QUEUE.MD` §0](../../../docs/QUEUE.MD#0-next-task-stack-queue_stackcsv)** (CSV columns).
2. Have a **completed** audit report path: e.g. **`docs/reports/AUDIT_REPORT_<date>_<slug>.md`** matching **[`FULL_AUDIT.md` §8.2](../../../docs/FULL_AUDIT.md#82-required-sections-in-order)**.

## Inputs

| Field | Source |
|-------|--------|
| **Findings table** | `## Findings` in the report — columns: Finding ID, Category ID, Severity, Location, Summary, Remediation, Status |
| **Filter** | Only **`Status`** = **Open** (or empty) — skip Fixed / Accepted risk unless user asks to track accepted risks as docs-only rows |

## ID and batch rules

| Report field | CSV mapping |
|--------------|-------------|
| **Finding ID** | e.g. `AUD-APPSEC-003` → **`id`** use **`FB-AUD-###`** or project convention; **`audit_id`** column = original Finding ID or audit code |
| **Category ID** | **`batch`** — use theme codes already in repo (e.g. **AUD-SEC**, **AUD-OPS**) or **`AUD-MISC`** |
| **Severity** | Map to **`priority`**: Critical/High→**HIGH**, Medium→**MEDIUM**, Low/Info→**LOW** (adjust if repo policy differs) |
| **`kind`** | `hardening`, `platform`, `tech_debt`, `change`, `reliability` — pick from **[`QUEUE.MD` §1.1](../../../docs/QUEUE.MD#11-kind-work-type)** |
| **`phase`** | Usually **B** for operator-facing; **C** for platform — align with existing **FB-AUD-*** rows |

## `agent_task` (mandatory)

Each CSV row **`agent_task`** must **stand alone**:

1. **Goal** — one sentence from the finding + report context.  
2. **Acceptance criteria** — bullet or numbered list (what “done” means).  
3. **Constraints** — link to **`docs_refs`** for deep context; do not require reading the full report for basic execution.  
4. **Traceability** — last line: `Traceability: <Finding ID> from <report filename>.`

## `stack_order`

- Assign **`stack_order`** so **next Open work** has the **smallest** number among Open rows (renumber existing Open rows if inserting at top).  
- Remove **`_QUEUE_EMPTY_`** sentinel when adding Open rows; restore if backlog becomes empty.

## Mirror `QUEUE_ARCHIVE.MD`

- If this repo uses **§2.5** post-audit tables, **append** a narrative row per new ID (match columns in archive) **or** add a single summary row referencing the report — follow project norms in **[`add-to-queue`](../add-to-queue/SKILL.md)**.  
- Update **`QUEUE.MD` §2** snapshot table if Open counts change.

## Do not

- **Duplicate** the entire audit report into **`agent_task`** — summarize.  
- **Drop** severity or category — preserve in **`audit_id`** / **`batch`**.  
- **Merge** unrelated findings into one row without user approval (default: **one row per Open finding**).

## After

- Run **`scripts/generate_queue_stack.py`** only if you maintain CSV via that script — otherwise edit CSV directly.  
- Link the **audit report path** in **`docs_refs`** for each new row.

## References

- [`docs/FULL_AUDIT.md`](../../../docs/FULL_AUDIT.md) — **§8** report shape, **§8.4** skills  
- [`docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD`](../../../docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD) — design brainstorm  
- [`.cursor/skills/add-to-queue/SKILL.md`](../add-to-queue/SKILL.md) — queue editing checklist

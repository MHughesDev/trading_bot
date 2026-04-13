# BS-006 — Audit report → queue stack *(brainstorm)*

**Brainstorm ID:** BS-006  
**Status:** ideas only — non-binding until promoted into the **[queue system](../QUEUE_SCHEMA.md)**  
**Related:** [`docs/FULL_AUDIT.md`](../FULL_AUDIT.md) · [`docs/reports/AUDIT_REPORT_TEMPLATE.md`](../reports/AUDIT_REPORT_TEMPLATE.md) · skills **`draft-audit-report`**, **`audit-report-to-queue`**

---

## Why this exists

Full audits produce **findings**; the **queue system** (`QUEUE_STACK.csv` + `QUEUE_ARCHIVE.MD`) tracks **executable work**. The gap is: how do we move from a **narrative audit report** to **self-contained `agent_task` rows** without losing severity, traceability, or splitting work badly?

This file is a **conversation starter** — not a spec.

---

## Design questions *(brainstorm)*

| Topic | Options / tensions |
|-------|-------------------|
| **Granularity** | One queue row per **finding** vs one row per **theme** (e.g. “CORS hardening”) vs one **epic** with sub-rows later. |
| **IDs** | Reuse **`AUD-*`** / **`FB-AUD-*`** style vs new prefix **`AUDIT-`** vs map **`Finding ID`** from report → **`audit_id`** column in CSV. |
| **Priority** | Map severity (Critical→HIGH) vs business impact vs “quick win” flag in **`summary_one_line`**. |
| **`agent_task`** | Must restate **goal + acceptance** so implementer never **requires** the PDF/report — only **`docs_refs`** link back. |
| **Done criteria** | Row closes when code merged vs when verified in staging vs when re-audit passes. |
| **Drift** | If report is updated after queue rows exist, **append** new rows or **revise** existing IDs — need a rule. |

---

## Flow *(sketch)*

1. Run **`FULL_AUDIT.md`** §4 categories → fill §2 / §5 in playbook.  
2. Emit **§8** standalone **`AUDIT_REPORT_*.md`** (or use **`draft-audit-report`**).  
3. **`audit-report-to-queue`:** parse **## Findings** table → append **`QUEUE_STACK.csv`** rows + mirror **`QUEUE_ARCHIVE.MD`** §2.5-style table if needed.  
4. Implement top of stack; close rows; optional **follow-up audit** slice.

---

## Open questions for you

- Should **every** Open finding become a row, or only **High+** / only **actionable**?  
- Do we want a **single epic parent** row (e.g. **FB-AUD-P1**) and children rows, or flat list?  
- Should **`audit-report-to-queue`** **delete** findings from the report when queued, or **never** edit the report (append-only history)?

---

## Promotion

When this brainstorm converges, add a short row in **`QUEUE_ARCHIVE.MD`** or a **queue system** note in **`FULL_AUDIT.md` §8** — no duplicate of this entire file in the backlog.

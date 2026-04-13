---
name: draft-audit-report
description: Turn raw full-audit outputs into a polished standalone audit report matching docs/FULL_AUDIT.md §8 and docs/reports/AUDIT_REPORT_TEMPLATE.md. Use after running a full or partial audit per FULL_AUDIT.md.
---

# Draft audit report

**Purpose:** Produce a **high-quality, standalone** audit report that mirrors **[`docs/FULL_AUDIT.md`](../../../docs/FULL_AUDIT.md) §8** and **[`docs/reports/AUDIT_REPORT_TEMPLATE.md`](../../../docs/reports/AUDIT_REPORT_TEMPLATE.md)**.

## Inputs

| Input | Required |
|-------|----------|
| **§0.2** repository profile | From the audit session |
| **Per-category results** | Verdict + findings + evidence from each **§4** category executed (may be rough notes) |
| **§5 playbook findings table** | If already filled in `FULL_AUDIT.md` |
| **Commit SHA / tag** | Audited revision |

## Outputs

1. **One markdown file** at **`docs/reports/AUDIT_REPORT_<YYYY-MM-DD>_<slug>.md`** (create `docs/reports/` if missing).  
2. **Sections** — strictly follow **[§8.2](../../../docs/FULL_AUDIT.md#82-required-sections-in-order)** in `FULL_AUDIT.md`:  
   - `# Audit report` (title + date)  
   - `## Metadata`  
   - `## Executive summary`  
   - `## Category results` (`### <ID> — <Name>` per category)  
   - `## Findings` (unified table with **Finding ID** like `AUD-G-001`)  
   - `## N/A categories`  
   - `## Residual gaps`  
   - `## Recommended next steps`  
   - `## Sign-off` (optional)

## Rules

1. **Map categories** to **§4** IDs: **G**, **SEC-REPO**, **SUP**, **STATIC**, **CORR**, **TEST**, **APPSEC**, **INFRA**, **CONT**, **DATA**, **PERF**, **REL**, **OBS**, **OPS**, **PRIV**, **DOC**, **ML**, **RLS**.
2. **Severity** for findings: Critical / High / Medium / Low / Info — consistent with **§0.3** master prompt.
3. **Evidence:** paths, workflow names, commands — **never** paste secrets or live tokens.
4. **Executive summary:** state **Pass / Pass with findings / Blocked** explicitly.
5. **Quality bar (§8.3):** every High/Critical finding has evidence; every finding has remediation hint.
6. **Cross-link** to [`FULL_AUDIT.md`](../../../docs/FULL_AUDIT.md) and the audited commit in **Metadata**.
7. If the user asks for **“fantastic”** depth: expand **Category results** with **strengths + gaps** per subsection; add a **Risk register** subsection under Executive summary only if the org wants it (optional).

## After drafting

- Optionally run **`audit-report-to-queue`** to promote **Open** findings to **`QUEUE_STACK.csv`**.
- Commit the report under **`docs/reports/`** with the same PR as playbook updates if applicable.

## References

- [`docs/FULL_AUDIT.md`](../../../docs/FULL_AUDIT.md) — **§8** deliverable spec  
- [`docs/reports/AUDIT_REPORT_TEMPLATE.md`](../../../docs/reports/AUDIT_REPORT_TEMPLATE.md) — skeleton  
- [`docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD`](../../../docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD) — design notes

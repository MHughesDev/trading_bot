# Open Questions

A living register of questions the system has not yet resolved. This is not a scratchpad —
it is the durable record of every consequential fork the project has faced, how it was decided,
and what evidence decided it.

## When to add an entry

Add an open question **whenever you compare important parts of the system or weigh trade-offs
between variations** — choosing a data store, an API to integrate, a pattern, a boundary between
components. If a decision is worth making deliberately, the question behind it is worth recording here.

## How entries move

1. **Open** — the question is raised. Record the options on the table and what would decide between them.
2. **Researching** — a research brief is underway (link it).
3. **Resolved** — the question is answered. Record the answer, the evidence that settled it, and the
   ADR or spec that now carries the decision.

Never delete a resolved question. The point of this file is that six months from now, anyone can see
not just *what* was decided but *why*, and which alternatives were already ruled out.

---

## Register

> The rows below are **examples — delete them** along with the other example artifacts. They are
> kept consistent with the example research brief, ADR, spec, and plan so the cross-references resolve.

| ID | Question | Status | Options Weighed | Resolution | Evidence / Links |
|----|----------|--------|-----------------|------------|------------------|
| Q-1 | Which auth provider fits a multi-tenant B2B app with a 2-person team? | Resolved | Auth0 vs. Clerk vs. custom JWT | Clerk | [research](./research/example-auth-provider-comparison.md) → [ADR-0001](./adr/0001-example-use-clerk-for-auth.md) |
| Q-2 | Does Clerk's SOC 2 Type II scope cover the Organizations feature specifically, or only core auth? | Open | — | — | Raised by [research §Open Questions](./research/example-auth-provider-comparison.md); affects [FEAT-001 §7.1](./specs/FEAT-001-user-authentication.md) |
| Q-3 | Can Clerk's EU data residency be contractually locked per-account (not just per-tenant)? | Open | Per-account vs. per-tenant residency | — | Raised by research; gates the EU-residency provisioning task in the [plan](./plans/example-task-management-app.md) |
| Q-4 | At what MAU/ARR does a Clerk → Auth0 migration become economically justified? | Open | Stay on Clerk vs. migrate to Auth0 | — | Raised by research; revisit at first $1M ARR milestone |
| Q-5 | Which email provider should deliver notifications (assignment + due-date reminders)? | Open | TBD (research brief needed) | — | Gates Milestone 3 of the [plan](./plans/example-task-management-app.md); write a research brief before specifying FEAT-003 |

> Number questions sequentially: `Q-1`, `Q-2`, … Append new ones at the end; never renumber.

---

## Entry Detail (optional)

For questions that need more than a table row, add a section below. Keep the table as the index.
The example below corresponds to Q-5 above — delete it with the rest of the example artifacts.

### Q-5: Which email provider should deliver notifications?

**Status:** Open
**Raised:** 2024-03-05

**Why it matters:** Notifications (assignment alerts and due-date reminders) are a launch success
condition. The provider choice affects deliverability, cost at scale, and the FEAT-003 spec that
cannot be written until this is resolved. It gates Milestone 3 of the plan entirely.

**Options weighed:**
- Transactional API (e.g. Postmark/SendGrid/Resend) — fast to integrate, per-email pricing, strong deliverability
- Self-hosted SMTP — no per-email cost, but deliverability and maintenance burden fall on a 2-person team
- The auth provider's built-in email — simplest, but may not support custom templated notifications

**What would decide it:** A research brief comparing deliverability SLAs, pricing at expected
notification volume, and template/scheduling support. Resolve before writing FEAT-003.

**Resolution:** [Filled in when resolved — the answer, and the research/ADR that carries it.]

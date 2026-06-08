# Example Formal Plan — Delete This File After Reading

> This file demonstrates the formal plan format end-to-end, including: the Derived From section
> with full artifact links, dependency-ordered milestones with Source columns, tasks with NOT
> STARTED status labels and source traces, pre-identified BLOCKED examples for all three valid
> blocked reasons, the coverage check table, and the Derivation Status field. Every task cites
> a spec acceptance criterion or architecture component so the derivation is auditable.

---

# Task Management App — Launch Plan

**Date:** 2024-03-05
**Type:** Formal
**Author:** Agent
**Status:** Active
**Derivation Status:** Current

## Goal

Ship a working multi-tenant task management app to the first 10 paying customers within 6 months,
with core task CRUD, workspace isolation, and email notifications functional at launch.

Derived from artifact success conditions: "first 10 paying customers within 6 months" and "core
task flow operational at launch." Not invented.

## Derived From

Every milestone and task below traces to one of these source artifacts. A task with no source
does not belong in this plan.

- Artifact: [docs/artifact.md](../artifact.md) — success conditions SC-1 through SC-5 and failure modes FM-1 through FM-4
- Architecture: [docs/architecture.md](../architecture.md) — §2 Components (Auth → Workspace → Task → Notification dependency chain), §4 External Dependencies
- Specs:
  - [FEAT-001 User Authentication](../specs/FEAT-001-user-authentication.md)
  - DATA-001 Workspace Schema *(to be created before Milestone 2 begins)*
  - FEAT-002 Core Task Flow *(to be created before Milestone 2 begins)*
  - FEAT-003 Email Notifications *(to be created before Milestone 3 begins)*
  - INTG-001 Stripe Billing *(to be created before Milestone 5 begins)*
- ADRs:
  - [ADR-0001 Use Clerk for Authentication](../adr/0001-example-use-clerk-for-auth.md)
- Research:
  - [Auth provider comparison](../research/example-auth-provider-comparison.md)
  - Email provider comparison *(research brief needed before Milestone 3)*
- Open questions: [open-questions.md](../open-questions.md) — Q-2, Q-3 (Clerk compliance), Q-5 (email provider)

## Scope

**In scope:**
- User authentication and multi-tenant workspace management (FEAT-001, DATA-001)
- Core task CRUD: create, assign, update status, archive (FEAT-002)
- Email notifications: assignment alert, due-date reminder (FEAT-003)
- Subscription billing with Stripe (INTG-001)

**Out of scope (directly from artifact and spec non-goals):**
- Mobile apps — web only at launch (FEAT-001 §2.2 / artifact non-goal)
- SSO/SAML enterprise login (FEAT-001 §2.2.A — deferred post-launch)
- Public API for third-party integrations (artifact non-goal)
- Time tracking (artifact non-goal)
- Account deletion and multi-device session management (FEAT-001 §2.2.D, §2.2.E)

## Dependencies

- Auth provider selected: resolved → ADR-0001 (Clerk)
- Clerk Pro plan provisioned with EU data residency before EU pilot customer onboarding
- Email provider selected: unresolved → open-questions.md Q-5 (gates Milestone 3)
- DATA-001 workspace schema spec approved before Milestone 2 begins
- FEAT-002 task spec approved before Milestone 2 task implementation begins
- FEAT-003 notifications spec approved before Milestone 3 begins
- INTG-001 Stripe spec approved before Milestone 5 begins

## Risks

Derived from artifact failure modes (FM) and open questions (Q):

- **FM-1: Auth integration takes longer than estimated** → Mitigation: 2-hour Clerk POC already
  completed; 2-day integration estimate is verified (research §Clerk "Developer experience")
- **FM-2: Multi-tenant data isolation bug ships to production** → Mitigation: FEAT-001 §6.1.C
  (403 enforcement) is a hard acceptance criterion with an automated test; pre-launch QA pass
  uses a second synthetic workspace to verify cross-workspace isolation explicitly
- **FM-3: Clerk SDK breaking change lands mid-development** → Mitigation: pin Clerk SDK version
  at project start; review changelog before any upgrade; budget 1–2 days for upgrade work
  per major version
- **FM-4: Email provider SLA causes notification delivery delays** → Mitigation: covered by
  FEAT-003 (to be specified); acceptance criterion will include a delivery time SLA
- **Q-2 risk: Clerk SOC 2 scope does not cover Organizations** → Mitigation: resolve Q-2 before
  first enterprise customer conversation; if unresolved, default to Auth0 Enterprise for any
  customer requiring attestation on multi-tenancy

## Milestones

Ordered by dependency chain derived from architecture §2 (Auth must exist before Workspace;
Workspace must exist before Task; Task must exist before Notifications). Milestone 4 (Beta)
is independent of Milestone 5 (Paid) in implementation but gates it in time.

| # | Milestone | Source | Success Signal |
|---|-----------|--------|----------------|
| 1 | Auth + Workspaces | FEAT-001 (all §6.1); architecture §2 (foundational layer) | A new user can sign up, verify email, and land on their workspace dashboard |
| 2 | Core Task Flow | FEAT-002 (all §6.1); DATA-001 (all §6.1); architecture §2 (Task layer) | A user can create, assign, update status, and archive a task in their workspace |
| 3 | Notifications | FEAT-003 (all §6.1); architecture §2 (Notification layer) | Assigned user receives email within 60 seconds; due-date reminder fires on schedule |
| 4 | Beta Launch | Artifact SC-3 "3 pilot customers active with real tasks" | 3 pilot workspaces active; each has created at least 5 real tasks |
| 5 | Paid Launch | INTG-001 (all §6.1); Artifact SC-5 "10 paying customers" | 10 active paid workspaces; billing confirmed in Stripe dashboard |

## Coverage Check

This table was produced by running Step 5 of `procedures/add-plan.md`. Every spec acceptance
criterion in the Derived From section maps to at least one task below. All gaps are noted.

| Source | AC / Success Condition | Covered by task in milestone |
|--------|----------------------|------------------------------|
| FEAT-001 §6.1.A | Email sign-up under 2 min | M1: "Implement email sign-up flow" |
| FEAT-001 §6.1.B | Google OAuth sign-up | M1: "Implement Google OAuth" |
| FEAT-001 §6.1.C | 403 on unauthorized workspace | M1: "Enforce workspace route protection" |
| FEAT-001 §6.1.D | Redirect preservation after login | M1: "Implement login redirect preservation" |
| FEAT-001 §6.1.E | Rate-limit after 5 failed logins | M1: "Implement login rate limiting" |
| FEAT-001 §6.1.F | "Email taken" error within 1s | M1: "Implement sign-up error states" |
| FEAT-001 §6.1.G | External redirect param ignored | M1: "Implement sign-up error states" |
| FEAT-001 §6.1.H | No Clerk errors exposed to user | M1: "Implement sign-up error states" |
| FEAT-001 §6.1.I | No-workspace redirect to onboarding | M1: "Handle no-workspace edge case" |
| DATA-001 §6.1.x | Workspace schema (TBD when spec exists) | M1: "Write DATA-001 spec" (creates the source) |
| FEAT-002 §6.1.x | Task CRUD (TBD when spec exists) | M2: "Write FEAT-002 spec" (creates the source) |
| Artifact SC-1 | "Core task flow operational at launch" | M2 entirely |
| Artifact SC-3 | "3 pilot customers active" | M4: onboarding and bug bash |
| Artifact SC-5 | "10 paying customers" | M5: billing integration and launch |

> **Coverage note:** DATA-001 and FEAT-002 acceptance criteria are TBD because those specs are
> not yet written. The tasks "Write DATA-001 spec" and "Write FEAT-002 spec" in Milestone 1 and 2
> create those sources. The coverage check must be re-run once those specs are approved and this
> plan must be re-derived (Derivation Status → STALE) at that point.

## Tasks by Milestone

### Milestone 1: Auth + Workspaces

All Milestone 1 tasks are unblocked. Milestone 1 has no upstream dependency.

- `NOT STARTED` Integrate Clerk SDK with the app; configure middleware for route protection (→ FEAT-001 §5.1.A, ADR-0001 §Rationale)
- `NOT STARTED` Implement email/password sign-up form with inline validation (→ FEAT-001 §5.1, §5.5.B, §5.5.C, §6.1.A)
- `NOT STARTED` Implement Google OAuth sign-up and login path (→ FEAT-001 §5.2, §6.1.B)
- `NOT STARTED` Implement email verification pending screen and resend flow (→ FEAT-001 §5.4.C)
- `NOT STARTED` Implement login with redirect param preservation and external-domain guard (→ FEAT-001 §5.3, §5.4.F, §6.1.D, §6.1.G)
- `NOT STARTED` Implement workspace route protection returning 403 with in-app error page (→ FEAT-001 §5.4.D, §6.1.C)
- `NOT STARTED` Implement login rate limiting (5 attempts → 30s lockout) (→ FEAT-001 §3.2.A, §5.5.D, §6.1.E)
- `NOT STARTED` Implement sign-up error states: email taken, Clerk 5xx, expired verification link (→ FEAT-001 §5.4.A, §5.5.A, §5.5.F, §6.1.F, §6.1.H)
- `NOT STARTED` Handle no-workspace state: redirect to /onboarding/create-workspace on login (→ FEAT-001 §5.4.E, §6.1.I)
- `NOT STARTED` Write DATA-001 workspace schema spec (→ architecture §2 Workspace layer — must exist before Milestone 2)
- `BLOCKED:TESTING` Verify email sign-up end-to-end in a running browser environment (→ FEAT-001 §6.1.A) — requires live Clerk sandbox with a real email inbox

### Milestone 2: Core Task Flow

Milestone 2 is BLOCKED:UPSTREAM on DATA-001 and FEAT-002 being approved (last two tasks of M1
and the first task of M2). Do not begin implementation tasks until those specs are approved.

- `NOT STARTED` Write FEAT-002 core task flow spec (→ architecture §2 Task layer; artifact SC-1 "core task flow operational")
- `NOT STARTED` Implement task schema migrations per DATA-001 (→ DATA-001 §3 — blocked on DATA-001 approval)
- `NOT STARTED` Implement task creation, assignment, and status update flows (→ FEAT-002 §5.1, §6.1 — blocked on FEAT-002 approval)
- `NOT STARTED` Implement task archive and restore flows (→ FEAT-002 §5.2, §6.1 — blocked on FEAT-002 approval)
- `NOT STARTED` Implement task list view with status filters (→ FEAT-002 §4, §6.1 — blocked on FEAT-002 approval)
- `BLOCKED:TESTING` Verify task CRUD end-to-end in a running browser environment (→ FEAT-002 §6.1) — requires running application with a seeded workspace

### Milestone 3: Notifications

Milestone 3 is BLOCKED:UPSTREAM on the email provider research brief (Q-5) and FEAT-003 spec.

- `NOT STARTED` Research and select an email provider (→ open-questions.md Q-5 — write research brief; produces ADR)
- `NOT STARTED` Write FEAT-003 email notifications spec (→ artifact "assignment reminders" — blocked on email provider decision)
- `NOT STARTED` Implement assignment notification trigger (→ FEAT-003 §5.1, §6.1 — blocked on FEAT-003 approval)
- `NOT STARTED` Implement due-date reminder scheduled job (→ FEAT-003 §5.2, §6.1 — blocked on FEAT-003 approval)
- `BLOCKED:TESTING` Verify assignment email delivers within 60 seconds in staging (→ FEAT-003 §6.1) — requires live email provider in staging environment

### Milestone 4: Beta Launch

Milestone 4 is blocked on Milestones 1–3 being complete or validly blocked.

- `NOT STARTED` Provision Clerk Pro plan with EU data residency before first EU customer (→ ADR-0001 §Consequences, Q-3) — `BLOCKED:HUMAN` requires account owner to upgrade Clerk plan and configure EU region
- `NOT STARTED` Onboard 3 pilot customers manually with walkthrough call (→ artifact SC-3) — `BLOCKED:HUMAN` requires scheduling with pilot customers
- `BLOCKED:TESTING` Bug bash: walk through every spec acceptance criterion in each pilot workspace (→ FEAT-001 §6.1, FEAT-002 §6.1, FEAT-003 §6.1) — requires pilot customers using real workspaces

### Milestone 5: Paid Launch

Milestone 5 is BLOCKED:UPSTREAM on Milestone 4 being complete (pilot validation required before
charging customers) and INTG-001 spec being approved.

- `NOT STARTED` Write INTG-001 Stripe billing integration spec (→ artifact SC-5 "10 paying customers")
- `NOT STARTED` Implement Stripe subscription checkout and webhook handling (→ INTG-001 §5, §6.1 — blocked on INTG-001 approval)
- `NOT STARTED` Implement plan enforcement: restrict feature access on expired subscription (→ INTG-001 §3, §6.1 — blocked on INTG-001 approval)
- `BLOCKED:HUMAN` Launch marketing page and announce beta-to-paid transition (→ artifact SC-5) — requires copywriting and design approval

## Open Questions

Questions that could affect this plan if their answers change:

- [ ] Q-2: Does Clerk's SOC 2 scope cover Organizations? If not, may require ADR amendment before M4.
- [ ] Q-3: Can Clerk EU data residency be contracted per-account? Affects M4 provisioning task.
- [ ] Q-5: Which email provider will be selected? Gates M3 entirely.

## Change Log

| Date | Change | Author |
|------|--------|--------|
| 2024-03-05 | Initial draft | Agent |
| 2024-03-08 | Added ADR-0001 to Derived From after Clerk decision; re-ran coverage check | Agent |

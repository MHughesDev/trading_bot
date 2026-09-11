# ADR-0025: Enforce research invariants at platform services — research cutoff, Desk project, scoped tokens, report validation

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)
**Extends:** ADR-0019, ADR-0020, ADR-0021 (Set J invariants)

## Context

Set J enforces its invariants inside the evaluation engine:
- INV-1: every trial counted;
- INV-2: sealed distributions;
- INV-3: no significance without a null and a trial count;
- the one-shot holdout vault.

ADR-0023 kept the agent honest mostly by restricting its tool profile. Under
ADR-0024 the agent has bash and code in a sandbox, so it can compute anything on
any data it can read, and it can ignore any prompt. A tool-profile or prompt-level
rule is no longer a guarantee.

## Decision

Every research invariant is enforced by a platform service that the agent's token
cannot bypass:

| Invariant | Enforcing service |
|---|---|
| Holdout untouched | **Data API research cutoff** per project. Research tokens never receive data with `event_time > cutoff`; the cutoff is immutable once an Experiment exists; only the vault-gate service reads past it |
| Live questions don't contaminate holdouts | **Desk project** per user (cutoff = now). Its Experiments are exploratory only (G0–G2), with no G3 or vault. Research projects refuse live and post-cutoff reads |
| INV-1 | The evaluation service counts every evaluation-counted job at submission, whatever the client. Submission is idempotent by manifest hash |
| INV-2 / INV-3 | Unchanged Set J engine; no best-member endpoint exists |
| Pre-registration | `create_experiment` refuses without a registered hypothesis and a current `data_qc` grade ≥ C |
| Research authority only | **Scoped session tokens.** The `research:*` scopes only; `trade:*`, `automation:*`, `model.alias:*`, `skills.admit` and `data.holdout` are never minted for agent sessions |
| Reports cite evidence | **`final_report` validator** service: ids exist, numbers match their artifacts, epistemic classes are consistent, and a dossier is required past G3 |
| Skills verified | Skill registry admission (ADR-0028) |
| Spend | LLM proxy and job-service budgets (ADR-0024, ADR-0030) |

Hooks and prompts provide fast feedback and teach good behaviour. They are never
the guarantee.

## Rationale

This is the design choice the recent quant-agent literature converges on:
separate proposal, deterministic audit, evaluation and holdout access, and test
the auditor itself against planted violations (arXiv 2608.25348). It also keeps
the platform honest for every client (UI, MCP, SDK), not just the agent.

## Consequences

- `sessions` (migration 0033) gains `scopes`, `project_id` and `expires_at`.
  Middleware checks the scopes on every route.
- The Data API, job service and experiment endpoints gain project context.
- The Desk project is a new concept for users. Live Q&A goes there automatically.
- The exploration ledger records Desk reads, so a later research hypothesis on the
  same variables is flagged `post_hoc`.

## Alternatives Considered

- **Prompt and hook enforcement only.** It can be bypassed by construction.
- **No cutoff, rely on the vault only.** The agent could read the holdout period
  through the Data API while exploring.
- **Holdout as a random block instead of the most recent period.** It would leak
  through autocorrelation and regime continuity. The most recent period, with the
  Desk split, is the honest option.

## References

- BS-007 [03 §2](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/03_RUNTIME.MD) and
  [06 §3](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD)
- [AGENT-001](../Specs/AGENT-001-agent-runtime.md)
- [DATA-005](../Specs/DATA-005-data-api-v2.md)
